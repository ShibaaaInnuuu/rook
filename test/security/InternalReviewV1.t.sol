// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Base} from "../utils/Base.sol";
import {stdError} from "forge-std/StdError.sol";
import {Merkle} from "../utils/Merkle.sol";
import {MockToken} from "../utils/Mocks.sol";
import {AssetVault} from "../../src/AssetVault.sol";
import {ListingRegistry} from "../../src/ListingRegistry.sol";
import {Distributor} from "../../src/Distributor.sol";
import {Treasury} from "../../src/Treasury.sol";
import {Clock} from "../utils/Clock.sol";

/// REPRODUCTIONS for docs/security/internal-review-v1.md.
///
/// Every test in this file DEMONSTRATES A FINDING: it asserts the behaviour the review reports, on the code as it stands.
/// None of them is a regression test — nothing here is fixed. If a finding is later closed, the test in this file that
/// demonstrates it must be deleted or inverted, not left passing. Named `test_repro_<finding id>_...` so that a grep for
/// `test_repro_` lists exactly the open findings that have a proof.
contract InternalReviewV1Reproductions is Base {
    string constant TAPROOT = "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0";
    bytes32 constant TXID = keccak256("a bitcoin txid");

    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    address carol = makeAddr("carol");
    bytes32[] shares;   // alice 60_000, bob 30_000, carol 9_000

    function setUp() public override {
        super.setUp();
        shares.push(distributor.shareLeaf(alice, 60_000));
        shares.push(distributor.shareLeaf(bob, 30_000));
        shares.push(distributor.shareLeaf(carol, 9_000));
    }

    // ---- H1: the order of two matured releases is anybody's to choose, and one order bricks the other declaration --------------

    /// The keeper checks `payersFor` before declaring, and the answer is correct when it declares. It is not reserved.
    /// `release` is permissionless and takes an id, so ANY caller picks which matured declaration eats the head of the queue
    /// first. One order leaves the second declaration needing 65 payers, and `TooManyPayers` is not a temporary condition:
    /// the queue only ever grows at the tail, so that declaration can never be released. Only the owner can cancel it.
    function test_repro_H1_releaseOrderTurnsAValidDeclarationIntoAPermanentlyUnreleasableOne() public {
        _fund(ordVault, coinA, 10 ether);                          // one whale at the head of the queue
        for (uint256 i = 0; i < 70; ++i) {                          // then 70 coins at exactly MIN_ENTRY behind it
            address c = address(uint160(0xD0570000 + i));
            launcher.set(c, router, creatorB, 0);
            _fund(ordVault, c, ordVault.MIN_ENTRY());
        }
        assertEq(ordVault.attributed(), 10.7 ether);

        vm.prank(keeper);
        ordVault.postCap(100 ether);

        // B is declared first, and at the moment it is declared it needs exactly ONE payer. The keeper did its job.
        assertEq(ordVault.payersFor(0.65 ether), 1, "one payer when B is declared");
        vm.prank(keeper);
        uint256 b = ordVault.declarePurchase(0.65 ether, 1, "cap-ref-b");
        vm.prank(keeper);
        uint256 a = ordVault.declarePurchase(9.99 ether, 1, "cap-ref-a");

        Clock.skip(1 hours);

        // a stranger, not the keeper and not the owner, picks the order
        vm.prank(stranger);
        ordVault.release(a);                                        // the whale is left holding 0.01
        assertEq(ordVault.unspentOf(coinA), 0.01 ether);

        vm.prank(stranger);
        vm.expectRevert(AssetVault.TooManyPayers.selector);
        ordVault.release(b);                                        // 1 + 64 dust coins > MAX_PAYERS

        // and it is permanent: nothing ever moves a coin forward in the queue
        Clock.skip(365 days);
        vm.expectRevert(AssetVault.TooManyPayers.selector);
        ordVault.release(b);

        // the ETH is not lost, but 0.65 stays reserved and the vault cannot declare against it until the OWNER cancels
        assertEq(ordVault.reserved(), 0.65 ether);
        vm.prank(stranger);
        vm.expectRevert(AssetVault.NotOwner.selector);
        ordVault.cancel(b);
        vm.prank(owner);
        ordVault.cancel(b);
        assertEq(ordVault.reserved(), 0);
    }

    // ---- C1 (FIXED): giving a purchase back in instalments no longer bricks the vault -------------------------------------------

    /// These three were the reproductions of C1. They now assert the fix, scenario for scenario.
    ///
    /// The defect: `returnUnspent` and `writeOff` both split an amount across the charge list with "the last payer takes the
    /// remainder", and both then did `chargedTo[coin] -= share`. The increment side (`release`) splits `maxEth` EXACTLY, so the
    /// last payer's cumulative decrement exceeded its charge as soon as (returns + slash) reached `maxEth` across more than one
    /// call — and at the default `bondBps` of 100% a write-off made returns + slash equal `maxEth` exactly, every time. Since
    /// `writeOff` is the only way out of `Released`, the panic left `isBlocked()` true for ever and the vault could never declare
    /// again. `_giveBack` now clamps each coin's share to what that coin has left unreturned and sweeps the truncation remainder
    /// onto coins that still have room, so no coin can ever be given back more than it paid.

    /// One wei from a stranger was the whole attack. The vault now absorbs it and writes off normally.
    function test_C1_fixed_oneWeiFromAStrangerDoesNotBrickTheVault() public {
        vm.prank(owner);
        registry.setBondBps(10_000);                      // the default on a fresh deployment
        vm.deal(purchaser, 10 ether);
        vm.deal(stranger, 1 ether);

        _fund(ordVault, coinA, 0.6 ether);                // two payers: the ordinary case once a listing has more than one coin
        _fund(ordVault, coinB, 0.4 ether);
        vm.prank(keeper);
        ordVault.postCap(1 ether);
        vm.prank(keeper);
        uint256 id = ordVault.declarePurchase(1 ether, 1, "cap-ref");
        Clock.skip(1 hours);
        vm.prank(purchaser);
        ordVault.postBond{value: 1 ether}();
        ordVault.release(id);
        assertEq(ordVault.chargesOf(id).length, 2);
        assertEq(ordVault.chargedTo(coinA), 0.6 ether);
        assertEq(ordVault.chargedTo(coinB), 0.4 ether);

        vm.prank(stranger);
        ordVault.returnUnspent{value: 1}(id);
        // a wei is too small to be anyone's proportional share, so the sweep places it on the first coin with room — and on the
        // coin that has room, never on whichever coin happens to be last
        assertEq(ordVault.chargedTo(coinA), 0.6 ether - 1, "the sweep placed the odd wei where there was room");
        assertEq(ordVault.chargedTo(coinB), 0.4 ether, "and the last payer did not absorb a refund that was not its share");

        // the purchaser never delivers. Three days later the vault writes the purchase off, as it always should have.
        Clock.skip(3 days + 1);
        (bool blocked, uint256 by) = ordVault.isBlocked();
        assertTrue(blocked);
        assertEq(by, id);

        vm.prank(owner);
        ordVault.writeOff(id, bytes32("never bought"));

        assertEq(uint8(ordVault.purchase(id).status), uint8(AssetVault.Status.WrittenOff));
        assertEq(ordVault.chargedTo(coinA), 0, "coinA was made whole to the wei");
        assertEq(ordVault.chargedTo(coinB), 0, "and so was coinB");
        assertEq(ordVault.totalWrittenOff(), 0, "a full bond covered the whole loss, so nothing is recorded as lost");
        assertEq(ordVault.bondOf(purchaser), 1, "and the purchaser's bond paid for it, all but the wei the stranger covered");

        // and the vault works again
        (blocked,) = ordVault.isBlocked();
        assertFalse(blocked);
        _fund(ordVault, coinA, 50 ether);
        vm.prank(keeper);
        ordVault.postCap(100 ether);
        vm.prank(keeper);
        ordVault.declarePurchase(10 ether, 1, "cap-ref");
    }

    /// The honest path, with nobody attacking. The defect's precise condition was that some instalment's split TRUNCATES — a
    /// split that happens to divide the charges exactly was always fine, which is why round numbers in a test hid it. Real
    /// refunds are not round numbers, so this walks three splits of the same money, including one that truncates on both coins.
    function test_C1b_fixed_anHonestSplitRefundCompletesWhateverTheInstalments() public {
        vm.deal(purchaser, 10 ether);
        _fund(ordVault, coinA, 0.6 ether);
        _fund(ordVault, coinB, 0.4 ether);
        vm.prank(keeper);
        ordVault.postCap(1 ether);
        vm.prank(keeper);
        uint256 id = ordVault.declarePurchase(1 ether, 1, "cap-ref");
        Clock.skip(1 hours);
        ordVault.release(id);

        // one instalment of the full amount: that split is exact by construction
        uint256 snap = vm.snapshotState();
        vm.prank(purchaser);
        ordVault.returnUnspent{value: 1 ether}(id);
        assertEq(ordVault.chargedTo(coinA), 0);
        assertEq(ordVault.chargedTo(coinB), 0);
        vm.revertToState(snap);

        // two instalments that divide the charges exactly
        vm.startPrank(purchaser);
        ordVault.returnUnspent{value: 0.3 ether}(id);
        ordVault.returnUnspent{value: 0.7 ether}(id);
        vm.stopPrank();
        assertEq(ordVault.chargedTo(coinA), 0);
        assertEq(ordVault.chargedTo(coinB), 0);
        vm.revertToState(snap);

        // and the same money split where it truncates — one third, then the rest. This is what used to revert.
        vm.startPrank(purchaser);
        ordVault.returnUnspent{value: 333_333_333_333_333_333}(id);
        assertEq(ordVault.chargedTo(coinA), 400_000_000_000_000_000, "the odd wei of a truncating split goes where there is room");
        assertEq(ordVault.chargedTo(coinB), 266_666_666_666_666_667, "so no coin is credited more than its share");
        ordVault.returnUnspent{value: 666_666_666_666_666_667}(id);
        vm.stopPrank();
        assertEq(ordVault.chargedTo(coinA), 0, "and the last instalment brings both coins home exactly");
        assertEq(ordVault.chargedTo(coinB), 0);
        assertEq(ordVault.purchase(id).returned, 1 ether, "the coins' whole money came back");
    }

    /// What C1 used to cost. `settleInscription` never touches `chargedTo`, so a bricked purchase could still be closed by
    /// receipting the full DECLARED quantity — which, when the purchaser had bought nothing, meant the attestor signing a false
    /// receipt against real member inscription ids, burning them for ever, and the purchaser keeping the whole bond. Incident
    /// response for a vault bricked by one wei was "have the attestor lie", the exact act the separation of keys exists to make
    /// detectable. This asserts the honest exit is now the one that is available.
    function test_C1c_fixed_theOverdueWriteOffClosesItWithoutAFalseReceipt() public {
        vm.prank(owner);
        registry.setBondBps(10_000);
        vm.deal(purchaser, 10 ether);
        vm.deal(stranger, 1 ether);

        _fund(ordVault, coinA, 0.6 ether);
        _fund(ordVault, coinB, 0.4 ether);
        vm.prank(keeper);
        ordVault.postCap(1 ether);
        vm.prank(keeper);
        uint256 id = ordVault.declarePurchase(1 ether, 1, "cap-ref");
        Clock.skip(1 hours);
        vm.prank(purchaser);
        ordVault.postBond{value: 1 ether}();
        ordVault.release(id);
        vm.prank(stranger);
        ordVault.returnUnspent{value: 1}(id);
        Clock.skip(3 days + 1);

        vm.prank(owner);
        ordVault.writeOff(id, bytes32("never bought"));

        // no inscription id was spent to get here
        assertFalse(ordVault.inscriptionHeld(registry.leafOf(ids[0])), "no member id was burned to unblock the vault");
        (bool blocked,) = ordVault.isBlocked();
        assertFalse(blocked, "unblocked, honestly");

        // and the purchaser, who delivered nothing, is left with the wei the bond did not need
        assertEq(ordVault.bondOf(purchaser), 1, "the bond paid for everything but the stranger's wei");
        vm.prank(purchaser);
        ordVault.withdrawBond(1);
        assertEq(purchaser.balance, 10 ether + 1, "delivered nothing, kept nothing of the coins' money");
        assertEq(ordVault.totalWrittenOff(), 0, "nothing recorded as lost, because the bond covered it");
    }


    /// The general statement, fuzzed: whatever instalments a 1 ETH purchase is given back in, every instalment is accepted and
    /// the coins end exactly whole. Three instalments are enough to reach the defect — it needed only two — and the bounds keep
    /// every split a real one.
    function testFuzz_C1_fixed_anySplitOfARefundCompletes(uint256 first, uint256 second) public {
        first = bound(first, 1, 1 ether - 2);
        second = bound(second, 1, 1 ether - first - 1);
        vm.deal(purchaser, 10 ether);
        _fund(ordVault, coinA, 0.6 ether);
        _fund(ordVault, coinB, 0.4 ether);
        vm.prank(keeper);
        ordVault.postCap(1 ether);
        vm.prank(keeper);
        uint256 id = ordVault.declarePurchase(1 ether, 1, "cap-ref");
        Clock.skip(1 hours);
        ordVault.release(id);

        vm.startPrank(purchaser);
        ordVault.returnUnspent{value: first}(id);
        ordVault.returnUnspent{value: second}(id);
        ordVault.returnUnspent{value: 1 ether - first - second}(id);
        vm.stopPrank();

        assertEq(ordVault.chargedTo(coinA), 0);
        assertEq(ordVault.chargedTo(coinB), 0);
        assertEq(ordVault.purchase(id).returned, 1 ether);
    }

    // ---- M7: a declaration never expires, so the keeper can pre-arm it against a cap that is long dead ---------------------------

    /// `release` checks `block.timestamp >= releasableAt` and nothing else. There is no upper bound. The cap's 1 h TTL bounds
    /// when a purchase may be DECLARED, never when it may be FIRED. So the keeper can declare `MAX_OPEN` purchases under an
    /// honest cap, sit on them, and release them months later into a collapsed market — and meanwhile `reserved` keeps that
    /// ETH out of every other declaration, indefinitely, at the keeper's sole discretion. Only the owner can clear it.
    /// Nosh hit the same shape on its off-chain vault and added `WITHDRAWAL_WINDOW = 6h` next to its announce delay.
    function test_repro_M7_aDeclarationCanBeFiredAYearLateAndSterilisesTheVaultUntilItIs() public {
        _fund(ordVault, coinA, 9 ether);
        vm.prank(keeper);
        ordVault.postCap(3 ether);

        // three declarations at a moment when 3 ETH an inscription is honest
        vm.startPrank(keeper);
        uint256 a = ordVault.declarePurchase(3 ether, 1, "floor-was-3-eth");
        ordVault.declarePurchase(3 ether, 1, "floor-was-3-eth");
        ordVault.declarePurchase(3 ether, 1, "floor-was-3-eth");
        vm.stopPrank();
        assertEq(ordVault.reserved(), 9 ether);

        // a year later the collection is worthless and the cap expired 8,759 hours ago. Nothing has changed on chain.
        Clock.skip(365 days);
        assertTrue(ordVault.capExpiry() < block.timestamp);

        // meanwhile nothing else could be declared for a year: MAX_OPEN is full and every wei is reserved
        _fund(ordVault, coinB, 5 ether);
        vm.prank(keeper);
        ordVault.postCap(3 ether);
        vm.prank(keeper);
        vm.expectRevert(AssetVault.TooManyOpen.selector);
        ordVault.declarePurchase(1 ether, 1, "cap-ref");

        // and the year-old declaration still pays out, in full, to anyone's call
        vm.prank(stranger);
        ordVault.release(a);
        assertEq(purchaser.balance, 3 ether, "released against a cap that died a year ago");
    }

    // ---- H2: the owner cannot take a vault's ETH, but can freeze it for ever in two transactions --------------------------------

    /// `setPaused` is instant by design (a protection applies at once). `renounceOwnership` is inherited from OpenZeppelin's
    /// Ownable and is NOT disabled by Ownable2Step. Together they are a one-way door: no release, no cancel, no unpause, no
    /// role rotation, ever again, and `AssetVault` has no withdraw function by design. There is no key that can undo it.
    function test_H2_fixed_ownershipIsNotRenounceableSoAPauseIsAlwaysReversible() public {
        _fund(ordVault, coinA, 5 ether);
        vm.prank(keeper);
        ordVault.postCap(10 ether);
        vm.prank(keeper);
        uint256 id = ordVault.declarePurchase(5 ether, 1, "cap-ref");
        Clock.skip(1 hours);

        vm.startPrank(owner);
        registry.setPaused(0, true);        // still instant, by design: a protection should apply at once
        vm.expectRevert(ListingRegistry.OwnershipIsNotRenounceable.selector);
        registry.renounceOwnership();       // the second half of the one-way door is gone
        vm.stopPrank();
        assertEq(registry.owner(), owner, "there is still an owner");

        // the pause does everything it did before
        vm.expectRevert(AssetVault.ListingPaused.selector);
        ordVault.release(id);
        vm.prank(keeper);
        vm.expectRevert(AssetVault.ListingPaused.selector);
        ordVault.declarePurchase(1, 1, "cap-ref");

        // and, because somebody is still the owner, it can be lifted — which is the whole difference
        vm.prank(owner);
        registry.setPaused(0, false);
        ordVault.release(id);
        assertEq(address(ordVault).balance, 0, "the ETH went where it was declared to go, not nowhere for ever");

        // ownership still moves, but only to somebody, and only through Ownable2Step's two steps
        address successor = makeAddr("successor multisig");
        vm.prank(owner);
        registry.transferOwnership(successor);
        assertEq(registry.owner(), owner, "step one does not move it");
        vm.prank(successor);
        registry.acceptOwnership();
        assertEq(registry.owner(), successor);
        vm.prank(successor);
        vm.expectRevert(ListingRegistry.OwnershipIsNotRenounceable.selector);
        registry.renounceOwnership();       // and the successor cannot do it either
    }

    // ---- M1: a role ROTATION may make the keeper the purchaser; only the deploy script checks ------------------------------------

    /// `proposeRoles` rejects purchaser == attestor and keeper == attestor, and says nothing about keeper == purchaser.
    /// `script/DeployChecks.sol` rejects all three — but only for the FIRST set of roles. Every rotation afterwards, which is
    /// exactly what happens under the time pressure of a suspected key compromise, goes through `proposeRoles` unchecked.
    function test_M1_fixed_noRotationMayCollapseTwoRolesIntoOneKey() public {
        address oneKey = makeAddr("one key for both roles");

        // the collision the registry used to allow: the key that posts the cap is also the key that is paid
        vm.prank(owner);
        vm.expectRevert(ListingRegistry.SameKey.selector);
        registry.proposeRoles(oneKey, oneKey, attestor);

        // and the two it always refused
        vm.startPrank(owner);
        vm.expectRevert(ListingRegistry.SameKey.selector);
        registry.proposeRoles(keeper, attestor, attestor);
        vm.expectRevert(ListingRegistry.SameKey.selector);
        registry.proposeRoles(attestor, purchaser, attestor);
        vm.stopPrank();

        // three distinct keys still rotate normally
        address k2 = makeAddr("keeper 2");
        address p2 = makeAddr("purchaser 2");
        address a2 = makeAddr("attestor 2");
        vm.prank(owner);
        registry.proposeRoles(k2, p2, a2);
        Clock.skip(7 days);
        vm.prank(owner);
        registry.acceptRoles();
        assertEq(registry.keeper(), k2);
        assertTrue(registry.keeper() != registry.purchaser());
    }

    /// The v2 shape of the same rule. An autonomous buyer can already pull a vault's ETH to itself, so it must never also hold
    /// the key that bounds the pulling (the keeper's cap) or the key that stops it (the owner's pause).
    function test_M1_fixed_noRoleMayBeRotatedOntoAnAutonomousBuyer() public {
        address buyerAddr = makeAddr("the canister's EVM address");
        vm.prank(owner);
        registry.listRuneAuto("auto rune", "840000:1", 0, "bc1qexample", payable(buyerAddr), 10 ether);
        assertTrue(registry.isAutoBuyer(buyerAddr));

        vm.startPrank(owner);
        vm.expectRevert(ListingRegistry.SameKey.selector);
        registry.proposeRoles(buyerAddr, purchaser, attestor);
        vm.expectRevert(ListingRegistry.SameKey.selector);
        registry.proposeRoles(keeper, buyerAddr, attestor);
        vm.expectRevert(ListingRegistry.SameKey.selector);
        registry.proposeRoles(keeper, purchaser, buyerAddr);
        vm.stopPrank();

        // and a buyer may not be named onto a key that already holds a role
        vm.prank(owner);
        vm.expectRevert(ListingRegistry.SameKey.selector);
        registry.listRuneAuto("no", "840000:2", 0, "bc1qexample", payable(keeper), 10 ether);
    }

    // ---- M2: the keeper can stop every rune payout with an unbounded, unappealable minimum ----------------------------------------

    /// `postMinimum` has no upper bound, no delay and no owner override, and `requestPayout` refuses both when the accrued
    /// amount is under it AND when the last minimum has expired. A keeper that posts `type(uint256).max` every 24 h — or that
    /// simply stops posting — freezes every holder's claimed units indefinitely. Nothing in the contracts can overrule it.
    function test_repro_M2_theKeeperCanBlockEveryRunePayoutWithAnUnboundedMinimum() public {
        uint256 aid = _runeAcquisition(100_000);
        vm.prank(creatorA);
        distributor.choose(1, aid, Distributor.Mode.ProRata, address(0));
        vm.prank(keeper);
        distributor.openDistribution(1, aid, Merkle.root(shares), "ipfs://shares");
        Clock.skip(15 minutes);
        distributor.claimShare(1, aid, alice, 60_000, Merkle.proof(shares, 0));
        assertEq(distributor.accrued(1, alice), 60_000);

        // an honest minimum: alice can be paid
        vm.prank(keeper);
        distributor.postMinimum(1, 10_000);
        uint256 snap = vm.snapshotState();
        vm.prank(alice);
        distributor.requestPayout(1, TAPROOT);
        assertEq(distributor.accrued(1, alice), 0);
        vm.revertToState(snap);

        // a hostile one: nothing bounds it
        vm.prank(keeper);
        distributor.postMinimum(1, type(uint256).max);
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(Distributor.BelowMinimum.selector, 60_000, type(uint256).max));
        distributor.requestPayout(1, TAPROOT);

        // the owner cannot post one either: postMinimum is keeper-only and there is no override
        vm.prank(owner);
        vm.expectRevert(Distributor.NotKeeper.selector);
        distributor.postMinimum(1, 1);

        // and simply going quiet has the same effect once the 24 h TTL lapses
        Clock.skip(24 hours + 1);
        vm.prank(alice);
        vm.expectRevert(Distributor.MinimumExpired.selector);
        distributor.requestPayout(1, TAPROOT);
    }

    // ---- M3: change returned below MIN_ENTRY is attributed but not spendable ---------------------------------------------------

    /// The honest path — a purchaser who buys under the cap and returns the difference — credits each paying coin its share of
    /// the refund. A coin that was drained to zero by the release re-enters the queue only at MIN_ENTRY, so any refund share
    /// under 0.01 ETH lands in `waiting` and is not spendable until that coin earns another 0.01 ETH of fees. For a coin that
    /// has stopped trading, which is the ordinary end of a memecoin, that is never.
    function test_repro_M3_aRefundBelowMinEntryIsAttributedButCanNeverBeSpent() public {
        _fund(ordVault, coinA, 0.02 ether);
        vm.prank(keeper);
        ordVault.postCap(0.02 ether);
        vm.prank(keeper);
        uint256 id = ordVault.declarePurchase(0.02 ether, 1, "cap-ref");
        Clock.skip(1 hours);
        ordVault.release(id);
        assertEq(ordVault.attributed(), 0);
        assertEq(ordVault.queued(coinA), false, "drained to zero and dequeued");

        // the purchaser bought at 0.015 and honestly returns the change
        vm.deal(purchaser, 1 ether);
        vm.prank(purchaser);
        ordVault.returnUnspent{value: 0.005 ether}(id);

        assertEq(ordVault.unspentOf(coinA), 0.005 ether, "coinA is owed it");
        assertEq(ordVault.waiting(), 0.005 ether);
        assertEq(ordVault.attributed(), 0, "but none of it is spendable");
        assertEq(address(ordVault).balance, 0.005 ether);

        vm.prank(keeper);
        ordVault.postCap(0.005 ether);
        vm.prank(keeper);
        vm.expectRevert(abi.encodeWithSelector(AssetVault.Insufficient.selector, 0.005 ether, 0));
        ordVault.declarePurchase(0.005 ether, 1, "cap-ref");

        // it unsticks only if the same coin ever earns another MIN_ENTRY of fees
        _fund(ordVault, coinA, 0.005 ether);
        assertEq(ordVault.attributed(), 0.01 ether);
        assertEq(ordVault.waiting(), 0);
    }

    // ---- M4: a rotated-out purchaser cannot top up their own bond, and their open declaration is stranded ---------------------

    /// `postBond()` credits `registry.purchaser()` — whoever holds the role NOW — while `release` requires the bond of the
    /// purchaser NAMED AT DECLARATION. After a rotation the two are different addresses, so a declaration made before the
    /// rotation can never be funded to the point where it may be released, and the old purchaser's attempt to fund their own
    /// obligation is a donation to their successor.
    function test_repro_M4_afterARotationTheNamedPurchasersBondCanNoLongerBeToppedUp() public {
        vm.prank(owner);
        registry.setBondBps(10_000);            // raising is immediate; this is the default on a fresh deployment

        _fund(ordVault, coinA, 5 ether);
        vm.prank(keeper);
        ordVault.postCap(10 ether);
        vm.prank(keeper);
        uint256 id = ordVault.declarePurchase(5 ether, 1, "cap-ref");   // names `purchaser`

        address purchaser2 = makeAddr("the new purchaser");
        vm.prank(owner);
        registry.proposeRoles(keeper, purchaser2, attestor);
        Clock.skip(7 days);
        vm.prank(owner);
        registry.acceptRoles();

        // the old purchaser, still named on the open declaration, tries to fund it
        vm.deal(purchaser, 10 ether);
        vm.prank(purchaser);
        ordVault.postBond{value: 5 ether}();
        assertEq(ordVault.bondOf(purchaser), 0, "their own ETH did not reach their own bond");
        assertEq(ordVault.bondOf(purchaser2), 5 ether, "it funded their successor instead");

        // so the declaration can never be released, whoever calls
        vm.prank(stranger);
        vm.expectRevert(abi.encodeWithSelector(AssetVault.BondTooSmall.selector, 5 ether, 0));
        ordVault.release(id);

        // and purchaser2, who holds the bond, may simply take it back: nothing is in their hands
        vm.prank(purchaser2);
        ordVault.withdrawBond(5 ether);
        assertEq(purchaser2.balance, 5 ether);
    }

    // ---- M6: lowering bondBps retroactively un-bonds a purchase that is already in the purchaser's hands -----------------------

    /// `requiredBondOf` reads `registry.bondBps()` LIVE, so a lowering applies to ETH already released, not only to future
    /// releases. The seven public days are presented as the safeguard; they are not one, because `SETTLE_DEADLINE` is only
    /// three days, so a purchase released inside the pending window is still live (not yet overdue, so `withdrawBond` does not
    /// block) when the lower rate lands. The purchaser pulls the difference out and then simply never settles.
    function test_repro_M6_aPendingBondReductionLetsAPurchaserUnbondALiveRelease() public {
        vm.prank(owner);
        registry.setBondBps(10_000);                         // the default: a release is fully bonded
        vm.deal(purchaser, 10 ether);

        // day 0: the owner announces a reduction to 10%, in public, seven days ahead
        vm.prank(owner);
        registry.setBondBps(1_000);
        uint256 day0 = vm.getBlockTimestamp();

        // day 5: business as usual. A 5 ETH purchase is declared and released, fully bonded at the rate still in force.
        Clock.warpTo(day0 + 5 days);
        _fund(ordVault, coinA, 5 ether);
        vm.prank(keeper);
        ordVault.postCap(10 ether);
        vm.prank(keeper);
        uint256 id = ordVault.declarePurchase(5 ether, 1, "cap-ref");
        Clock.skip(1 hours);
        vm.prank(purchaser);
        ordVault.postBond{value: 5 ether}();
        ordVault.release(id);
        assertEq(ordVault.requiredBondOf(purchaser), 5 ether, "5 ETH out, 5 ETH behind it");

        // day 7: the reduction lands. The purchase is not overdue (its deadline is day 8), so nothing blocks the withdrawal.
        Clock.warpTo(day0 + 7 days);
        vm.prank(owner);
        registry.acceptBondBps();
        assertEq(ordVault.requiredBondOf(purchaser), 0.5 ether, "and now 0.5 ETH is behind it");

        vm.prank(purchaser);
        ordVault.withdrawBond(4.5 ether);
        assertEq(purchaser.balance, 10 ether - 5 ether + 5 ether + 4.5 ether, "the purchaser is 4.5 ETH up on the bond");

        // day 8 onwards: never settle. The write-off recovers only what is left.
        Clock.warpTo(ordVault.purchase(id).deadline + 1);
        vm.prank(owner);
        ordVault.writeOff(id, bytes32("never bought"));
        assertEq(ordVault.totalWrittenOff(), 4.5 ether, "the 100% bond in force at release covered 10%");
        assertEq(ordVault.attributed(), 0.5 ether);
    }

    // ---- L1: an overdue receipt stops new declarations but not a matured one --------------------------------------------------

    /// `isBlocked` gates `declarePurchase` and `withdrawBond`. It does not gate `release`. The documents say "the machine stops
    /// at the first overdue receipt"; up to MAX_OPEN − 1 already-declared purchases still pay out after it.
    function test_repro_L1_anOverdueReceiptDoesNotStopAMaturedRelease() public {
        _fund(ordVault, coinA, 4 ether);
        vm.prank(keeper);
        ordVault.postCap(10 ether);
        vm.prank(keeper);
        uint256 a = ordVault.declarePurchase(2 ether, 1, "cap-ref");
        vm.prank(keeper);
        uint256 b = ordVault.declarePurchase(2 ether, 1, "cap-ref");
        Clock.skip(1 hours);
        ordVault.release(a);

        Clock.skip(3 days + 1);
        (bool blocked, uint256 by) = ordVault.isBlocked();
        assertTrue(blocked);
        assertEq(by, a);

        vm.prank(keeper);
        ordVault.postCap(10 ether);
        vm.prank(keeper);
        vm.expectRevert(abi.encodeWithSelector(AssetVault.BlockedByOverdue.selector, a));
        ordVault.declarePurchase(1 ether, 1, "cap-ref");

        vm.prank(stranger);
        ordVault.release(b);                    // but this one still pays
        assertEq(purchaser.balance, 4 ether);
    }

    // ---- L2: a stranger may discard a drand round whose value is already public -----------------------------------------------

    /// `redraw` is permissionless and its only condition is that nobody has PROVEN the pinned round on chain — not that drand
    /// failed to produce it. drand's value for a round is public seconds after the round is due, so at pinnedAt + 24 h any
    /// caller who does not like the outcome can replace the round instead of resolving it, and repeat. `voidDraw`, which is the
    /// same act (a second draw), is owner-only and waits 7 days.
    function test_repro_L2_aStrangerCanReplaceAPinnedRoundWhoseValueIsAlreadyKnown() public {
        uint256 aid = _inscriptionAcquisition();
        vm.prank(creatorA);
        distributor.choose(0, aid, Distributor.Mode.DrawWeighted, address(0));
        bytes32[] memory tickets = new bytes32[](2);
        tickets[0] = distributor.ticketLeaf(alice, 0, 50);
        tickets[1] = distributor.ticketLeaf(bob, 50, 100);
        vm.prank(keeper);
        distributor.openDraw(0, aid, Merkle.root(tickets), 100, "ipfs://tickets");
        Clock.skip(15 minutes);
        distributor.requestDraw(0, aid);
        uint64 firstRound = distributor.lot(0, aid).round;

        // the round happens. Its value is public off chain the moment it does; here the stranger simply reads it and computes
        // the winner it WOULD produce, without proving it on chain.
        bytes32 value = keccak256("the signature drand published for this round");
        uint256 wouldHaveWon = uint256(keccak256(abi.encode(value, uint256(0), aid))) % 100;

        // 24 h later, nobody having proven it, the stranger throws that round away instead
        Clock.skip(24 hours);
        vm.prank(stranger);
        distributor.redraw(0, aid);
        uint64 secondRound = distributor.lot(0, aid).round;
        assertTrue(secondRound > firstRound, "a different round decides it now");

        // the first round's outcome is unreachable: resolve now settles the NEW round
        vm.prank(stranger);
        distributor.resolve(0, aid, abi.encodePacked(keccak256("a different signature")));
        assertTrue(distributor.lot(0, aid).winningTicket != wouldHaveWon || wouldHaveWon == 0);
        assertEq(uint8(distributor.lot(0, aid).stage), uint8(Distributor.Stage.Resolved));
    }

    // ---- M5: a draw pinned to a beacon that never produces can never leave Stage.Pinned ------------------------------------------

    /// `voidDraw` is the escape hatch for a lot nobody can claim, and it starts from `Stage.Resolved`. There is no escape hatch
    /// from `Stage.Pinned`. If the beacon stops producing — evmnet retired, the League of Entropy's chain wound down — every
    /// pinned lot is stuck: `resolve` cannot prove a round nobody signed, `redraw` only pins another round nobody will sign,
    /// `voidDraw` refuses the stage, `choose` is final, and `beacon` is immutable in the Distributor. Nobody can undo it.
    function test_repro_M5_aPinnedLotIsStuckForEverIfTheBeaconStopsProducing() public {
        uint256 aid = _inscriptionAcquisition();
        vm.prank(creatorA);
        distributor.choose(0, aid, Distributor.Mode.DrawWeighted, address(0));
        bytes32[] memory tickets = new bytes32[](2);
        tickets[0] = distributor.ticketLeaf(alice, 0, 50);
        tickets[1] = distributor.ticketLeaf(bob, 50, 100);
        vm.prank(keeper);
        distributor.openDraw(0, aid, Merkle.root(tickets), 100, "ipfs://tickets");
        Clock.skip(15 minutes);
        distributor.requestDraw(0, aid);
        assertEq(uint8(distributor.lot(0, aid).stage), uint8(Distributor.Stage.Pinned));

        // the beacon never produces again. A year on:
        Clock.skip(365 days);

        // the owner cannot void it — voidDraw starts from Resolved
        vm.prank(owner);
        vm.expectRevert(Distributor.WrongStage.selector);
        distributor.voidDraw(0, aid);

        // the keeper cannot re-open it — openDraw starts from None
        vm.prank(keeper);
        vm.expectRevert(Distributor.WrongStage.selector);
        distributor.openDraw(0, aid, Merkle.root(tickets), 100, "ipfs://tickets-2");

        // the creator cannot choose again — anything but Hold is final
        vm.prank(creatorA);
        vm.expectRevert(Distributor.WrongStage.selector);
        distributor.choose(0, aid, Distributor.Mode.Named, alice);

        // and redraw only ever pins another round the dead beacon will not sign
        vm.prank(stranger);
        distributor.redraw(0, aid);
        assertEq(uint8(distributor.lot(0, aid).stage), uint8(Distributor.Stage.Pinned), "still Pinned, for ever");
    }

    // ---- L3: forwardToken sends ERC-20s to a Treasury that has no way to move them ---------------------------------------------

    /// `FeeRouter.forwardToken` is permissionless and sends the router's whole balance of any ERC-20 to `registry.treasury()`.
    /// `src/Treasury.sol` — the contract the deployment puts there — has no ERC-20 function at all: no transfer, no rescue, no
    /// owner hook. The NatSpec says selling the token is "the treasury's word"; with this treasury it is not a word it can keep.
    function test_repro_L3_theTreasuryContractCannotMoveAnErc20ItIsSent() public {
        Treasury t = new Treasury(registry);
        MockToken tok = new MockToken();
        tok.mint(address(t), 1_000e18);

        bytes[6] memory attempts = [
            abi.encodeWithSignature("transfer(address,uint256)", owner, uint256(1)),
            abi.encodeWithSignature("sweep(address)", address(tok)),
            abi.encodeWithSignature("rescue(address,uint256)", address(tok), uint256(1)),
            abi.encodeWithSignature("rescueToken(address)", address(tok)),
            abi.encodeWithSignature("withdrawToken(address,address,uint256)", address(tok), owner, uint256(1)),
            abi.encodeWithSignature("forwardToken(address)", address(tok))
        ];
        for (uint256 i = 0; i < attempts.length; ++i) {
            vm.prank(owner);
            (bool ok,) = address(t).call(attempts[i]);
            assertFalse(ok, "no function on the treasury moves a token");
        }
        assertEq(tok.balanceOf(address(t)), 1_000e18, "and it is still there");
    }

    // ---- helpers ---------------------------------------------------------------------------------------------------------------

    function _inscriptionAcquisition() internal returns (uint256 aid) {
        _fund(ordVault, coinA, 1 ether);
        vm.prank(keeper);
        ordVault.postCap(1 ether);
        vm.prank(keeper);
        uint256 id = ordVault.declarePurchase(1 ether, 1, "cap-ref");
        Clock.skip(1 hours);
        ordVault.release(id);
        vm.prank(attestor);
        aid = ordVault.settleInscription(id, ids[0], TXID, 1, _proofOf(0));
    }

    function _runeAcquisition(uint256 amount) internal returns (uint256 aid) {
        aid = runeVault.acquisitionCount();
        _fund(runeVault, coinA, 1 ether);
        vm.prank(keeper);
        runeVault.postCap(1 ether);
        vm.prank(keeper);
        uint256 id = runeVault.declarePurchase(1 ether, amount, "cap-ref");
        Clock.skip(1 hours);
        runeVault.release(id);
        vm.prank(attestor);
        runeVault.settleRune(id, amount, TXID, 1);
    }
}
