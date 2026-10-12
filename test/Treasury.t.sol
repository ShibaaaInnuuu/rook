// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Base} from "./utils/Base.sol";
import {Treasury} from "../src/Treasury.sol";
import {AssetVault} from "../src/AssetVault.sol";
import {Clock} from "./utils/Clock.sol";

/// docs/token/protocol-token-v1.md §2. The treasury's twenty per cent may go into a vault and nowhere else.
contract TreasuryTest is Base {
    Treasury t;
    address token = makeAddr("protocol token");
    address tokenRouter = makeAddr("the protocol token's router");
    address tokenCreator = makeAddr("the protocol token's creator");

    function setUp() public override {
        super.setUp();
        launcher.set(token, tokenRouter, tokenCreator, 0);
        t = new Treasury(registry);
        vm.startPrank(owner);
        registry.proposeTreasury(address(t));
        Clock.skip(7 days);
        registry.acceptTreasury();
        t.setToken(token);
        vm.stopPrank();
        vm.deal(address(t), 100 ether);
    }

    // ---- what it is for ----

    function test_theKeeperMovesTreasuryEthIntoAVaultAttributedToTheToken() public {
        vm.prank(keeper);
        t.fund(0, 4 ether);
        assertEq(address(t).balance, 96 ether);
        assertEq(address(ordVault).balance, 4 ether);
        assertEq(ordVault.contributedBy(token), 4 ether);
        assertEq(ordVault.attributed(), 4 ether);
        assertEq(t.totalFunded(), 4 ether);
        assertEq(t.fundedTo(0), 4 ether);
        // and it works for a rune listing exactly the same way
        vm.prank(keeper);
        t.fund(1, 1 ether);
        assertEq(runeVault.contributedBy(token), 1 ether);
        assertEq(t.fundedTo(1), 1 ether);
    }

    function test_feesArriveByPlainTransfer_andAreCounted() public {
        Treasury fresh = new Treasury(registry);
        vm.deal(stranger, 3 ether);
        vm.prank(stranger);
        (bool ok,) = address(fresh).call{value: 3 ether}("");
        assertTrue(ok);
        assertEq(fresh.totalReceived(), 3 ether);
        assertEq(address(fresh).balance, 3 ether);
    }

    // ---- what nobody can do ----

    function test_thereIsNoWithdraw_forAnyone() public {
        bytes4[6] memory sels = [bytes4(keccak256("withdraw()")), bytes4(keccak256("withdraw(uint256)")),
            bytes4(keccak256("withdraw(address,uint256)")), bytes4(keccak256("sweep(address)")),
            bytes4(keccak256("rescue(address,uint256)")), bytes4(keccak256("transfer(address,uint256)"))];
        address[5] memory who = [owner, keeper, purchaser, attestor, stranger];
        for (uint256 w = 0; w < who.length; ++w) {
            for (uint256 i = 0; i < sels.length; ++i) {
                vm.prank(who[w]);
                (bool ok,) = address(t).call(abi.encodeWithSelector(sels[i], who[w], 1 ether));
                assertFalse(ok);
            }
        }
        assertEq(address(t).balance, 100 ether);
    }

    function test_onlyTheKeeperFunds() public {
        address[3] memory who = [owner, purchaser, stranger];
        for (uint256 i = 0; i < who.length; ++i) {
            vm.prank(who[i]);
            vm.expectRevert(Treasury.NotKeeper.selector);
            t.fund(0, 1 ether);
        }
    }

    function test_theVaultAddressComesFromTheRegistry_soItCannotBeAimedAnywhereElse() public {
        vm.prank(keeper);
        vm.expectRevert();                    // the registry has no listing 7: it reverts before any ETH moves
        t.fund(7, 1 ether);
        assertEq(address(t).balance, 100 ether);
    }

    function test_onlyTheTreasuryMayAttributeMoneyToTheTokenInAVault() public {
        vm.deal(stranger, 1 ether);
        vm.prank(stranger);
        vm.expectRevert(AssetVault.NotTreasury.selector);
        ordVault.depositFrom{value: 1 ether}(token);
        vm.prank(router);                     // a router is not the treasury either
        vm.expectRevert(AssetVault.NotTreasury.selector);
        ordVault.depositFrom{value: 1 ether}(token);
        assertEq(ordVault.contributedBy(token), 0);
    }

    function test_ethCannotBeAttributedToACoinTheLauncherDoesNotKnow() public {
        vm.prank(address(t));
        vm.expectRevert(AssetVault.UnknownCoin.selector);
        ordVault.depositFrom{value: 1 ether}(makeAddr("a coin nobody launched"));
    }

    // ---- the token is named once, in public ----

    function test_theTokenIsSetOnce_byTheOwner_andMustBeACoinTheLauncherKnows() public {
        Treasury fresh = new Treasury(registry);
        vm.prank(stranger);
        vm.expectRevert(Treasury.NotOwner.selector);
        fresh.setToken(token);
        vm.prank(owner);
        vm.expectRevert(Treasury.UnknownCoin.selector);
        fresh.setToken(makeAddr("not a coin"));
        vm.prank(owner);
        fresh.setToken(token);
        assertEq(fresh.token(), token);
        vm.prank(owner);
        vm.expectRevert(Treasury.TokenAlreadySet.selector);
        fresh.setToken(coinA);
    }

    function test_nothingCanBeFundedBeforeATokenIsNamed() public {
        Treasury fresh = new Treasury(registry);
        vm.deal(address(fresh), 1 ether);
        vm.prank(owner);
        registry.proposeTreasury(address(fresh));
        Clock.skip(7 days);
        vm.prank(owner);
        registry.acceptTreasury();
        vm.prank(keeper);
        vm.expectRevert(Treasury.TokenNotSet.selector);
        fresh.fund(0, 1 ether);
    }

    // ---- the levers, and their limits ----

    function test_theOwnerCanStopFunding_andStoppingIsNotTaking() public {
        vm.prank(stranger);
        vm.expectRevert(Treasury.NotOwner.selector);
        t.setPaused(true);
        vm.prank(owner);
        t.setPaused(true);
        vm.prank(keeper);
        vm.expectRevert(Treasury.IsPaused.selector);
        t.fund(0, 1 ether);
        assertEq(address(t).balance, 100 ether);        // still here, still nobody's to take
        vm.prank(owner);
        t.setPaused(false);
        vm.prank(keeper);
        t.fund(0, 1 ether);
        assertEq(ordVault.contributedBy(token), 1 ether);
    }

    function test_aPausedListingIsNotFunded() public {
        vm.prank(owner);
        registry.setPaused(0, true);
        vm.prank(keeper);
        vm.expectRevert(Treasury.ListingPaused.selector);
        t.fund(0, 1 ether);
    }

    function test_itCannotFundMoreThanItHas_orNothingAtAll() public {
        vm.prank(keeper);
        vm.expectRevert(abi.encodeWithSelector(Treasury.Insufficient.selector, 101 ether, 100 ether));
        t.fund(0, 101 ether);
        vm.prank(keeper);
        vm.expectRevert(Treasury.ZeroAmount.selector);
        t.fund(0, 0);
    }

    function test_treasuryMoneyBuysForTheTokensHolders_andForNobodyElse() public {
        // coinA's own fees and the treasury's share sit in the same vault, each attributed to its own coin
        _fund(ordVault, coinA, 2 ether);
        vm.prank(keeper);
        t.fund(0, 3 ether);
        assertEq(ordVault.contributedBy(coinA), 2 ether);
        assertEq(ordVault.contributedBy(token), 3 ether);
        assertEq(ordVault.attributed(), 5 ether);
    }
}
