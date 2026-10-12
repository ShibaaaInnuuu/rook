// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Base} from "./utils/Base.sol";
import {AssetVault} from "../src/AssetVault.sol";
import {ListingRegistry} from "../src/ListingRegistry.sol";

/// docs/phase1.md F4: the purchaser's bond. Every rule it claims has a test that tries to break it.
contract BondTest is Base {
    function setUp() public override {
        super.setUp();
        vm.prank(owner);
        registry.setBondBps(10_000);          // raising is immediate
        vm.deal(purchaser, 100 ether);
    }

    function _declared(uint256 eth_) internal returns (uint256 id) {
        _fund(ordVault, coinA, eth_);
        vm.prank(keeper); ordVault.postCap(eth_);
        vm.prank(keeper); id = ordVault.declarePurchase(eth_, 1, "f");
        Clock.skip(1 hours);
    }

    function test_noBondNoRelease_andTheBondMustCoverWhatIsAlreadyOutToo() public {
        uint256 a = _declared(1 ether);
        vm.expectRevert(abi.encodeWithSelector(AssetVault.BondTooSmall.selector, 1 ether, 0));
        ordVault.release(a);

        vm.prank(purchaser); ordVault.postBond{value: 1.5 ether}();
        ordVault.release(a);
        assertEq(ordVault.outstanding(), 1 ether);

        uint256 b = _declared(1 ether);       // a second purchase needs 2 ETH of bond in all; 1.5 is not enough
        vm.expectRevert(abi.encodeWithSelector(AssetVault.BondTooSmall.selector, 2 ether, 1.5 ether));
        ordVault.release(b);
        ordVault.postBond{value: 0.5 ether}();   // anyone may add to it
        ordVault.release(b);
        assertEq(ordVault.outstanding(), 2 ether);
    }

    function test_aWriteOffPaysTheCoinsThatWereChargedFromTheBond_andRecordsOnlyWhatItDidNotCover() public {
        vm.prank(purchaser); ordVault.postBond{value: 0.6 ether}();
        vm.prank(owner); registry.setBondBps(10_000);
        // 60% bonded is only possible if the requirement is 60%: lower it the slow way
        vm.prank(owner); registry.setBondBps(6_000);
        Clock.skip(7 days);
        vm.prank(owner); registry.acceptBondBps();

        uint256 id = _declared(1 ether);
        ordVault.release(id);                                  // the purchaser now holds 1 ETH and never buys
        assertEq(ordVault.contributedBy(coinA), 1 ether);
        Clock.skip(3 days + 1);
        vm.prank(owner); ordVault.writeOff(id, bytes32("never bought"));

        assertEq(ordVault.bond(), 0, "the whole 0.6 was slashed");
        assertEq(ordVault.totalWrittenOff(), 0.4 ether, "only what the bond did not cover is lost");
        assertEq(ordVault.attributed(), 0.6 ether, "and the slashed ETH is spendable again, owed to the coin that paid");
        assertEq(ordVault.outstanding(), 0);
        assertEq(address(ordVault).balance, 0.6 ether);
    }

    function test_atOneHundredPercentTheCoinsAreMadeWhole() public {
        vm.prank(purchaser); ordVault.postBond{value: 1 ether}();
        uint256 id = _declared(1 ether);
        ordVault.release(id);
        Clock.skip(3 days + 1);
        vm.prank(owner); ordVault.writeOff(id, bytes32("x"));
        assertEq(ordVault.totalWrittenOff(), 0);
        assertEq(ordVault.attributed(), 1 ether, "theft costs the thief exactly what was taken");
    }

    function test_theBondCannotBeWithdrawnBelowWhatIsOutstanding_norWhileBlocked_norByAnyoneElse() public {
        vm.prank(purchaser); ordVault.postBond{value: 3 ether}();
        uint256 id = _declared(1 ether);
        ordVault.release(id);

        vm.prank(stranger); vm.expectRevert(AssetVault.NotPurchaser.selector); ordVault.withdrawBond(1 ether);
        vm.prank(owner); vm.expectRevert(AssetVault.NotPurchaser.selector); ordVault.withdrawBond(1 ether);
        vm.prank(keeper); vm.expectRevert(AssetVault.NotPurchaser.selector); ordVault.withdrawBond(1 ether);

        vm.prank(purchaser);
        vm.expectRevert(abi.encodeWithSelector(AssetVault.BondTooSmall.selector, 1 ether, 3 ether));
        ordVault.withdrawBond(2 ether + 1);
        uint256 before = purchaser.balance;
        vm.prank(purchaser); ordVault.withdrawBond(2 ether);   // down to exactly what is needed
        assertEq(purchaser.balance, before + 2 ether);

        Clock.skip(3 days + 1);      // the receipt is now overdue: the vault is blocked
        vm.prank(purchaser); ordVault.postBond{value: 5 ether}();
        vm.prank(purchaser);
        vm.expectRevert(abi.encodeWithSelector(AssetVault.BlockedByOverdue.selector, id));
        ordVault.withdrawBond(1 ether);
    }

    function test_aSettledPurchaseFreesTheBond_andUnspentEthReducesWhatIsOutstanding() public {
        vm.prank(purchaser); ordVault.postBond{value: 1 ether}();
        uint256 id = _declared(1 ether);
        ordVault.release(id);
        vm.prank(purchaser); ordVault.returnUnspent{value: 0.25 ether}(id);
        assertEq(ordVault.outstanding(), 0.75 ether);
        vm.prank(attestor); ordVault.settleInscription(id, ids[0], bytes32("txid"), 1, _idProof(0));
        assertEq(ordVault.outstanding(), 0);
        vm.prank(purchaser); ordVault.withdrawBond(1 ether);
        assertEq(ordVault.bond(), 0);
    }

    function test_anOwnerWhoDoesNothingCannotShelterTheBond_afterSevenMoreDaysAnyoneWritesOff() public {
        vm.prank(purchaser); ordVault.postBond{value: 1 ether}();
        uint256 id = _declared(1 ether);
        ordVault.release(id);
        Clock.skip(3 days + 1);
        vm.prank(stranger); vm.expectRevert(AssetVault.NotOwner.selector); ordVault.writeOff(id, bytes32("x"));   // the owner's, for now
        Clock.skip(7 days);
        vm.prank(stranger); ordVault.writeOff(id, bytes32("the owner did nothing"));
        assertEq(ordVault.bond(), 0);
        assertEq(ordVault.attributed(), 1 ether);
    }

    function test_theRequirementRisesAtOnce_fallsOnlyAfterSevenPublicDays_andOnlyTheOwnerSetsIt() public {
        assertEq(registry.bondBps(), 10_000);
        vm.prank(stranger); vm.expectRevert(); registry.setBondBps(0);
        vm.prank(owner); vm.expectRevert(ListingRegistry.BadBps.selector); registry.setBondBps(10_001);
        vm.startPrank(owner);
        registry.setBondBps(2_000);
        assertEq(registry.bondBps(), 10_000, "a lower requirement is only a proposal");
        vm.expectRevert(); registry.acceptBondBps();
        Clock.skip(7 days);
        registry.acceptBondBps();
        assertEq(registry.bondBps(), 2_000);
        registry.setBondBps(9_000);
        assertEq(registry.bondBps(), 9_000, "a higher one applies at once");
        vm.stopPrank();
    }

    function test_aNewRegistryStartsFullyBonded() public {
        assertEq(new ListingRegistry(owner, treasury).bondBps(), 10_000);
    }

    function _idProof(uint256 i) internal view returns (bytes32[] memory) {
        return Merkle.proof(idLeaves, i);
    }
}

import {Merkle} from "./utils/Merkle.sol";
import {Clock} from "./utils/Clock.sol";
