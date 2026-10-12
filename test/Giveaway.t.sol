// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Base} from "./utils/Base.sol";
import {Merkle} from "./utils/Merkle.sol";
import {MockToken} from "./utils/Mocks.sol";
import {Treasury} from "../src/Treasury.sol";
import {AssetVault} from "../src/AssetVault.sol";
import {Distributor} from "../src/Distributor.sol";
import {Clock} from "./utils/Clock.sol";

/// docs/token/protocol-token-v1.md. The whole path, end to end: fees reach the treasury, the treasury funds a vault attributed to
/// the protocol token, the vault buys an inscription through the machinery every coin uses, and the Distributor gives it to one of
/// the token's holders, weighted by balance, with nothing to enter.
///
/// The Distributor is NOT changed by any of this: the lot below is opened, challenged, pinned, resolved and claimed by exactly the
/// code a coin's own draw uses. What these tests fix is that the snapshot is of the TOKEN's holders and that the exclusions hold.
contract GiveawayTest is Base {
    string constant TAPROOT = "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0";
    bytes32 constant TXID = keccak256("a bitcoin txid");

    Treasury treasuryC;
    MockToken token;
    address tokenRouter = makeAddr("the token's router");
    address tokenCreator = makeAddr("the token's creator");

    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    address carol = makeAddr("carol");
    address late = makeAddr("a holder who bought after the snapshot");
    address team = makeAddr("the team's address");
    address pool = makeAddr("the token's liquidity pool");

    bytes32[] tickets;      // the snapshot: alice [0,60) bob [60,90) carol [90,100)

    function setUp() public override {
        super.setUp();
        token = new MockToken();
        launcher.set(address(token), tokenRouter, tokenCreator, 0);
        treasuryC = new Treasury(registry);
        vm.startPrank(owner);
        registry.proposeTreasury(address(treasuryC));
        Clock.skip(7 days);
        registry.acceptTreasury();
        treasuryC.setToken(address(token));
        vm.stopPrank();

        // Balances at the snapshot block. 60/30/10 among eligible holders; the excluded hold plenty and count for nothing.
        token.mint(alice, 600e18);
        token.mint(bob, 300e18);
        token.mint(carol, 100e18);
        token.mint(address(treasuryC), 5_000e18);
        token.mint(team, 2_000e18);
        token.mint(pool, 1_000e18);
        token.mint(address(distributor), 50e18);
        token.mint(address(ordVault), 50e18);

        tickets.push(distributor.ticketLeaf(alice, 0, 60));
        tickets.push(distributor.ticketLeaf(bob, 60, 90));
        tickets.push(distributor.ticketLeaf(carol, 90, 100));
    }

    // ---- helpers: the ordinary machinery, paid for by the treasury ----

    function _treasuryBuysAnInscription() internal returns (uint256 aid) {
        vm.deal(address(treasuryC), 10 ether);          // every coin's router sent its twenty per cent here
        vm.prank(keeper); treasuryC.fund(0, 1 ether);
        vm.prank(keeper); ordVault.postCap(1 ether);
        vm.prank(keeper); uint256 id = ordVault.declarePurchase(1 ether, 1, "floor: giveaway");
        Clock.skip(1 hours);
        ordVault.release(id);
        vm.prank(attestor);
        aid = ordVault.settleInscription(id, ids[0], TXID, 1, _proofOf(0));
    }

    function _openedGiveaway() internal returns (uint256 aid) {
        aid = _treasuryBuysAnInscription();
        vm.prank(tokenCreator);
        distributor.choose(0, aid, Distributor.Mode.DrawWeighted, address(0));
        vm.prank(keeper);
        distributor.openDraw(0, aid, Merkle.root(tickets), 100, "ipfs://the-token-snapshot");
    }

    function _resolved(uint256 aid) internal returns (uint256 t) {
        Clock.skip(15 minutes);
        distributor.requestDraw(0, aid);
        vm.prank(stranger);
        distributor.resolve(0, aid, abi.encodePacked(keccak256("the round's value")));
        t = distributor.lot(0, aid).winningTicket;
    }

    function _winner(uint256 t) internal view returns (address w, uint256 a, uint256 b, uint256 i) {
        return t < 60 ? (alice, uint256(0), uint256(60), uint256(0))
             : t < 90 ? (bob, uint256(60), uint256(90), uint256(1))
                      : (carol, uint256(90), uint256(100), uint256(2));
    }

    // ---- the path ----

    function test_theTreasurysFeesBuyAnAssetAndOneTokenHolderGetsIt() public {
        uint256 aid = _treasuryBuysAnInscription();
        AssetVault.Acquisition memory a = ordVault.acquisition(aid);
        assertEq(a.coin, address(token));               // charged to the protocol token, so it is the token's holders who draw
        assertEq(a.amount, 1);
        assertEq(ordVault.chargedTo(address(token)), 1 ether);

        vm.prank(tokenCreator);
        distributor.choose(0, aid, Distributor.Mode.DrawWeighted, address(0));
        vm.prank(keeper);
        distributor.openDraw(0, aid, Merkle.root(tickets), 100, "ipfs://the-token-snapshot");
        uint256 t = _resolved(aid);
        (address w, uint256 lo, uint256 hi, uint256 i) = _winner(t);
        vm.prank(stranger);                             // anyone may submit the winner's proof; the winner did nothing to enter
        distributor.claimWin(0, aid, w, lo, hi, Merkle.proof(tickets, i));
        assertEq(distributor.lot(0, aid).winner, w);
        vm.prank(w);
        distributor.setDestination(0, aid, TAPROOT);
        assertEq(distributor.lot(0, aid).destination, TAPROOT);
    }

    /// @notice The rule that makes a snapshot a snapshot. docs/token/protocol-token-v1.md §2.1.
    function test_aHolderWhoBuysAfterTheSnapshotBlockGetsNothing() public {
        uint256 aid = _openedGiveaway();
        vm.prank(alice);
        token.transfer(late, 500e18);                   // a huge balance, bought after the list was published
        assertEq(token.balanceOf(late), 500e18);
        uint256 t = _resolved(aid);
        // the late buyer is in no leaf: no range they can name has a proof, including the winning one
        (,, , uint256 i) = _winner(t);
        vm.expectRevert(Distributor.BadProof.selector);
        distributor.claimWin(0, aid, late, 0, 100, Merkle.proof(tickets, i));
        vm.expectRevert(Distributor.BadProof.selector);
        distributor.claimWin(0, aid, late, t, t + 1, Merkle.proof(tickets, i));
        // and the winner named in the published list still wins, whatever anyone bought afterwards
        (address w, uint256 lo, uint256 hi, uint256 wi) = _winner(t);
        distributor.claimWin(0, aid, w, lo, hi, Merkle.proof(tickets, wi));
        assertEq(distributor.lot(0, aid).winner, w);
    }

    /// @notice Every address in the exclusion table holds a balance here, and none of them can take the asset.
    function test_theExcludedHoldTokensAndWinNothing() public {
        uint256 aid = _openedGiveaway();
        uint256 t = _resolved(aid);
        (, , , uint256 i) = _winner(t);
        address[4] memory excluded = [address(treasuryC), team, pool, address(distributor)];
        for (uint256 k = 0; k < excluded.length; ++k) {
            assertGt(token.balanceOf(excluded[k]), 0);
            vm.expectRevert(Distributor.BadProof.selector);
            distributor.claimWin(0, aid, excluded[k], 0, 100, Merkle.proof(tickets, i));
        }
        // the treasury holds more than every eligible holder together and still wins nothing
        assertGt(token.balanceOf(address(treasuryC)), token.balanceOf(alice) + token.balanceOf(bob) + token.balanceOf(carol));
    }

    /// @notice If the snapshot finds nobody eligible there is no list to publish, so no draw can be opened and the asset waits.
    function test_aGiveawayWithNoEligibleHolderCannotBeOpened() public {
        uint256 aid = _treasuryBuysAnInscription();
        vm.prank(tokenCreator);
        distributor.choose(0, aid, Distributor.Mode.DrawWeighted, address(0));
        vm.startPrank(keeper);
        vm.expectRevert(Distributor.BadInput.selector);
        distributor.openDraw(0, aid, bytes32(0), 0, "ipfs://empty");        // no holders: no root
        vm.expectRevert(Distributor.BadInput.selector);
        distributor.openDraw(0, aid, Merkle.root(tickets), 0, "ipfs://empty");
        vm.stopPrank();
        assertEq(uint8(distributor.lot(0, aid).stage), uint8(Distributor.Stage.None));
        // nothing was drawn, nothing was lost: the asset is still the vault's, and a later snapshot can open the same lot
        vm.prank(keeper);
        distributor.openDraw(0, aid, Merkle.root(tickets), 100, "ipfs://a-later-snapshot");
        assertEq(uint8(distributor.lot(0, aid).stage), uint8(Distributor.Stage.Open));
    }

    function test_theChallengeWindowMustPassBeforeAnyoneCanPinTheDraw() public {
        uint256 aid = _openedGiveaway();
        uint64 ends = distributor.lot(0, aid).challengeEnds;
        assertEq(ends, uint64(vm.getBlockTimestamp() + 15 minutes));
        vm.expectRevert(abi.encodeWithSelector(Distributor.ChallengeWindow.selector, ends));
        distributor.requestDraw(0, aid);
        Clock.warpTo(ends - 1);
        vm.expectRevert(abi.encodeWithSelector(Distributor.ChallengeWindow.selector, ends));
        distributor.requestDraw(0, aid);
        Clock.warpTo(ends);
        distributor.requestDraw(0, aid);                                    // anyone, once the list has been public for 15 minutes
        assertEq(uint8(distributor.lot(0, aid).stage), uint8(Distributor.Stage.Pinned));
    }

    function test_theWinnerIsClaimedOnce_andTheLotCannotBeReDrawnAfterwards() public {
        uint256 aid = _openedGiveaway();
        uint256 t = _resolved(aid);
        (address w, uint256 lo, uint256 hi, uint256 i) = _winner(t);
        distributor.claimWin(0, aid, w, lo, hi, Merkle.proof(tickets, i));
        vm.expectRevert(Distributor.WrongStage.selector);
        distributor.claimWin(0, aid, w, lo, hi, Merkle.proof(tickets, i));
        vm.expectRevert(Distributor.WrongStage.selector);
        distributor.resolve(0, aid, abi.encodePacked(keccak256("another round")));
        vm.prank(keeper);
        vm.expectRevert(Distributor.WrongStage.selector);
        distributor.openDraw(0, aid, Merkle.root(tickets), 100, "ipfs://a-second-bite");
    }

    /// @notice docs/token/protocol-token-v1.md §2.2: a rune giveaway is not a draw. Every eligible holder is paid pro rata, and
    /// the excluded are absent from the list, so their share is not theirs to claim.
    function test_aRuneGiveawayIsSharedProRataAmongTheTokensHolders() public {
        vm.deal(address(treasuryC), 10 ether);
        vm.prank(keeper); treasuryC.fund(1, 1 ether);
        uint256 aid = runeVault.acquisitionCount();
        vm.prank(keeper); runeVault.postCap(1 ether);
        vm.prank(keeper); uint256 id = runeVault.declarePurchase(1 ether, 100_000, "floor: giveaway");
        Clock.skip(1 hours);
        runeVault.release(id);
        vm.prank(attestor);
        runeVault.settleRune(id, 100_000, TXID, 1);
        assertEq(runeVault.acquisition(aid).coin, address(token));

        bytes32[] memory shares = new bytes32[](3);
        shares[0] = distributor.shareLeaf(alice, 60_000);
        shares[1] = distributor.shareLeaf(bob, 30_000);
        shares[2] = distributor.shareLeaf(carol, 10_000);
        bytes32[] memory leaves = shares;
        vm.prank(tokenCreator);
        distributor.choose(1, aid, Distributor.Mode.ProRata, address(0));
        vm.prank(keeper);
        distributor.openDistribution(1, aid, Merkle.root(leaves), "ipfs://the-token-snapshot");
        Clock.skip(15 minutes);
        distributor.claimShare(1, aid, alice, 60_000, Merkle.proof(leaves, 0));
        assertEq(distributor.accrued(1, alice), 60_000);
        vm.expectRevert(Distributor.AlreadyClaimed.selector);                  // claim once
        distributor.claimShare(1, aid, alice, 60_000, Merkle.proof(leaves, 0));
        vm.expectRevert(Distributor.BadProof.selector);                        // the treasury is not in the list
        distributor.claimShare(1, aid, address(treasuryC), 60_000, Merkle.proof(leaves, 0));
    }
}
