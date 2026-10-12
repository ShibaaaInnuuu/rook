// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";

/// test/security/Surface.t.sol, for the NFT side: the compiled ABI of each NFT vault and of the registry, asserted EXACTLY. A vault
/// with no withdraw and a fate fixed at launch are claims about an ABI; a function added later — a setter for the policy, the
/// collection or the buyer, a release, an approve — fails here, not on a chain.
contract NftSurfaceTest is Test {
    function _surface(string memory artifact) internal view returns (string[] memory) {
        return vm.parseJsonKeys(vm.readFile(string.concat("out/", artifact)), ".methodIdentifiers");
    }

    function _assertExactly(string memory artifact, string memory expectedList) internal view {
        string[] memory actual = _surface(artifact);
        string[] memory expected = vm.split(expectedList, " ");
        assertEq(actual.length, expected.length, string.concat(artifact, ": the number of external functions changed"));
        for (uint256 i = 0; i < actual.length; ++i) {
            bool found;
            for (uint256 j = 0; j < expected.length; ++j) {
                if (keccak256(bytes(actual[i])) == keccak256(bytes(expected[j]))) { found = true; break; }
            }
            assertTrue(found, string.concat(artifact, " exposes ", actual[i], ", which is not on its list"));
        }
    }

    /// H12 (docs/v5/custody-hardening.md) added the cap's step bound and the owner's slow ceiling to both vaults: the nine names at
    /// the end of each list below.
    function test_theSweepVaultExposesExactlyWhatItMeansTo() public view {
        _assertExactly("NftSweepVault.sol/NftSweepVault.json", "CAP_TTL() CHALLENGE() DEAD() MAX_CANDIDATES() MAX_PAYERS() MIN_ENTRY() PIN_AHEAD() REDRAW_AFTER() VOID_AFTER() accounted() acquisition(uint256) acquisitionCount() attributed() beacon() burn(uint256) cancelDraw(uint256) capExpiry() capExpiryBlock() capFresh() capWei() chainId() chargedTo(address) chargesOf(uint256) claimWin(uint256,address,uint256,uint256,bytes32[]) collection() contributedBy(address) deposit(address) listingId() lot(uint256) onERC721Received(address,address,uint256,bytes) openDraw(uint256,bytes32,uint256,string) policy() postCap(uint256,string) queueAt(uint256) queueHead() queueLength() queued(address) redraw(uint256) registry() requestDraw(uint256) resolve(uint256,bytes) sweep(address,bytes,uint256[],uint256) sweepCount() ticketLeaf(address,uint256,uint256) unspentOf(address) voidDraw(uint256) waiting() CEILING_DELAY() MAX_STEP_BPS() MIN_POST_GAP() acceptCapCeiling() capCeiling() pendingCeiling() pendingCeilingAt() pendingCeilingAtBlock() proposeCapCeiling(uint256)");
    }

    function test_theRemoteVaultExposesExactlyWhatItMeansTo() public view {
        _assertExactly("NftRemoteVault.sol/NftRemoteVault.json", "CAP_TTL() CHALLENGE() DEAD() DELIVERY_DEADLINE() DESTINATION_CORRECTION() MAX_OPEN() MAX_PAYERS() MAX_QUANTITY() MIN_ENTRY() PIN_AHEAD() PUBLIC_CANCEL_AFTER() PUBLIC_WAIT() PUBLIC_WRITEOFF_AFTER() REDRAW_AFTER() SETTLE_DEADLINE() VOID_AFTER() accounted() acquisition(uint256) acquisitionCount() announce(uint256,uint256,string) attributed() beacon() buyer() cancelAnnouncement(uint256) cancelDraw(uint256) capExpiry() capExpiryBlock() capFresh() capWei() chainId() chargedTo(address) chargesOf(uint256) claimWin(uint256,address,uint256,uint256,bytes32[]) collection() contributedBy(address) deposit(address) flagOverdue(uint256) headroom() inFlight() isBlocked() listingId() lot(uint256) maxInFlight() openDraw(uint256,bytes32,uint256,string) openPurchases() policy() postCap(uint256,string) pull(uint256) purchase(uint256) purchaseCount() queueAt(uint256) queueHead() queueLength() queued(address) receipt(uint256,uint256,bytes32) receipted(uint256) recordBurn(uint256,bytes32) recordDelivery(uint256,bytes32) redraw(uint256) registry() requestDraw(uint256) reserved() resolve(uint256,bytes) returnFunds(uint256) setDestination(uint256,address) settleShort(uint256) ticketLeaf(address,uint256,uint256) totalWrittenOff() unspentOf(address) voidDraw(uint256) waiting() writeOff(uint256) CEILING_DELAY() MAX_STEP_BPS() MIN_POST_GAP() acceptCapCeiling() capCeiling() pendingCeiling() pendingCeilingAt() pendingCeilingAtBlock() proposeCapCeiling(uint256)");
    }

    /// No function changes a listing's chain, collection, policy, vault or buyer once it is made.
    function test_theRegistryExposesExactlyWhatItMeansTo() public view {
        _assertExactly("NftRegistry.sol/NftRegistry.json", "CHANGE_DELAY() acceptSweepTarget(address) beacon() count() isNative(uint256) isPaused(uint256) isSweepTarget(address) keeper() launcher() listNft(uint64,address,uint8,string,address,uint256) listed(bytes32) listing(uint256) listingRegistry() owner() proposeSweepTarget(address) ref(uint256) removeSweepTarget(address) setPaused(uint256,bool) targetReadyAt(address) targetReadyAtBlock(address) treasury() vaultOf(uint256) wire(address)");
    }

    /// docs/v4/nft.md §2.1: ListingRegistry gains exactly `nftRegistry()`, `wireNft(address)` and `reserveBuyer(address)` (and, from
    /// docs/audit/2026-09-26/fixes-B-C-keeper-draws.md, the keeper incident switch and the guardian). Everything
    /// the web app, the canister's `ret_listing` and golive-check decode is still here, under the same signature.
    function test_listingRegistryOnlyGainedTheReservationHook() public view {
        _assertExactly("ListingRegistry.sol/ListingRegistry.json", "CHANGE_DELAY() acceptBondBps() acceptBtcVault(uint256) acceptOwnership() acceptRoles() acceptRoot(uint256) acceptTreasury() attestor() bondBps() count() distributor() guardian() isAutoBuyer(address) isMember(uint256,string,bytes32[]) isPaused(uint256) keeper() keeperPaused() keeperResumedAt() keeperResumedAtBlock() launcher() leafOf(string) listInscriptions(string,string,bytes32,string) listInscriptionsAuto(string,string,bytes32,string,address,uint256) listRune(string,string,uint8,string) listRuneAuto(string,string,uint8,string,address,uint256) listing(uint256) namedKeeper() nftRegistry() owner() pauseKeeper() pendingBondAt() pendingBondBps() pendingBondReadyBlock() pendingBtcVault(uint256) pendingBtcVaultReadyBlock(uint256) pendingOwner() pendingRoles() pendingRolesReadyBlock() pendingRoot(uint256) pendingRootReadyBlock(uint256) pendingTreasury() pendingTreasuryAt() pendingTreasuryReadyBlock() proposeBtcVault(uint256,string) proposeRoles(address,address,address) proposeRoot(uint256,bytes32) proposeTreasury(address) purchaser() renounceOwnership() reserveBuyer(address) setBondBps(uint16) setGuardian(address) setPaused(uint256,bool) transferOwnership(address) treasury() unpauseKeeper() vaultOf(uint256) wire(address,address) wireNft(address)");
        // the struct the three decoders read, by selector: listing(uint256) -> (uint8,bool,bool,uint8,address,bytes32,string,string,string)
        string memory json = vm.readFile("out/ListingRegistry.sol/ListingRegistry.json");
        assertEq(vm.parseJsonString(json, ".methodIdentifiers['listing(uint256)']"), "84a184de");
    }

    function test_noVaultHasAWayOutByName() public view {
        string[2] memory artifacts = ["NftSweepVault.sol/NftSweepVault.json", "NftRemoteVault.sol/NftRemoteVault.json"];
        string[12] memory forbidden = ["withdraw", "rescue", "drain", "release", "approve", "setApproval", "transfer", "safeTransfer",
            "setPolicy", "setCollection", "setBuyer", "execute"];
        for (uint256 a = 0; a < artifacts.length; ++a) {
            string[] memory actual = _surface(artifacts[a]);
            for (uint256 i = 0; i < actual.length; ++i) {
                for (uint256 f = 0; f < forbidden.length; ++f) {
                    assertFalse(_startsWith(actual[i], forbidden[f]), string.concat(artifacts[a], " exposes ", actual[i]));
                }
            }
        }
    }

    function _startsWith(string memory s, string memory prefix) internal pure returns (bool) {
        bytes memory b = bytes(s);
        bytes memory p = bytes(prefix);
        if (p.length > b.length) return false;
        for (uint256 i = 0; i < p.length; ++i) if (b[i] != p[i]) return false;
        return true;
    }
}
