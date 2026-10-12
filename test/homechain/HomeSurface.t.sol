// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";

/// test/nft/NftSurface.t.sol, for the home chain: the compiled ABI of every new contract, asserted EXACTLY. "No key, no withdraw, no
/// owner lever, no signature" are claims about an ABI; a function added later — a setter for the reward or the markup, a rescue, an
/// `isValidSignature` that would let Seaport honour an order the vault never validated — fails here, not on a chain.
contract HomeSurfaceTest is Test {
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

    string constant BASE = "CAP_TTL() CHANGE_DELAY() DEAD() L2_GAS_LIMIT() L2_MAX_FEE() MAX_CANDIDATES() MAX_HINTS() MAX_PAYERS() MIN_ENTRY() acceptKeeper() accounted() acknowledge(uint256) acknowledged(uint256) acquisition(uint256) acquisitionCount() attributed() beacon() bridgeFee(uint256) buffer() buyPunk(uint256) buySeaport(bytes,uint256[],uint256) capExpiry() capExpiryBlock() capFresh() capWei() chargesOf(uint256) cheaperHint(uint256,uint256) checkOrders(bytes,uint256[]) collection() escrow() fund(uint256) fundedFast(uint256) fundedNative(uint256) guard() hintAsk(uint256) hints() inbox() isPunks() keeper() keeperReadyAt() keeperReadyAtBlock() onERC721Received(address,address,uint256,bytes) outbox() owner() parentBlockSeconds() pendingKeeper() policy() postCap(uint256,string) proceedsHeld() proposeKeeper(address) purchaseCount() queueAt(uint256) queueHead() queueLength() queued(uint256) receiver() reward() seaport() unspentOf(uint256) waiting()";

    function test_theRaffleVaultExposesExactlyWhatItMeansTo() public view {
        _assertExactly("HomeVaultRaffle.sol/HomeVaultRaffle.json", string.concat(BASE,
            " CHALLENGE() PIN_AHEAD() REDRAW_AFTER() VOID_AFTER() cancelDraw(uint256) claimWin(uint256,address,uint256,uint256,bytes32[]) draw(uint256) openDraw(uint256,bytes32,uint256,string) redraw(uint256) requestDraw(uint256) resolve(uint256,bytes) ticketLeaf(address,uint256,uint256) voidDraw(uint256)"));
    }

    function test_theBurnVaultExposesExactlyWhatItMeansTo() public view {
        _assertExactly("HomeVaultBurn.sol/HomeVaultBurn.json", string.concat(BASE, " burn(uint256)"));
    }

    function test_theYoyoVaultExposesExactlyWhatItMeansTo() public view {
        _assertExactly("HomeVaultYoyo.sol/HomeVaultYoyo.json", string.concat(BASE,
            " MAX_MARKUP_BPS() MIN_MARKUP_BPS() listing(uint256) listingTtl() markupBps() relist(uint256) settle(uint256)"));
    }

    function test_theWrapVaultExposesExactlyWhatItMeansTo() public view {
        _assertExactly("HomeVaultWrap.sol/HomeVaultWrap.json", string.concat(BASE, " ONE_ITEM() gatewayRouter() wrapper()"));
    }

    function test_theWrapperExposesExactlyWhatItMeansTo() public view {
        _assertExactly("HomeWrapper.sol/HomeWrapper.json", "DEAD() DELIVERY_DEADLINE() ONE_ITEM() PIN_AHEAD() REDRAW_AFTER() allowance(address,address) approve(address,uint256) balanceOf(address) beacon() burned() collection() custodian() decimals() deliver(uint256) freeAt(uint256) freeCount() guard() held() isPunks() item(uint256) itemCount() itemOf(uint256) name() nextToResolve() openTickets() parentBlockSeconds() pendingAt(uint256) pendingCount() pendingHead() redeem(address) redeemToBurn() redeemToRelist() redraw(uint256) refund(uint256) refunded(uint256) resolve(uint256,bytes) symbol() ticket(uint256) ticketCount() totalSupply() transfer(address,uint256) transferFrom(address,address,uint256) vault() wrap(uint256,address)");
    }

    function test_theGuardExposesExactlyWhatItMeansTo() public view {
        _assertExactly("HomeGuard.sol/HomeGuard.json", "MAX_RAISE() RAISE_INTERVAL() STAGE0_ITEMS() STAGE0_OUT_GLOBAL() STAGE0_OUT_PER_COLLECTION() STAGE0_OUT_VALUE() STAGE0_VALUE() TRIP_SPAN() WINDOW() admit(address,uint256,uint256) capOf(address) collectionOf(address) global() globalWindow() limits() out(address,uint256,uint256) owner() parentBlockSeconds() paused() raise(address,uint256,uint256,uint256) raiseGlobal(uint256,uint256) register(address,address) release(address,uint256,uint256) setLimits(uint256,uint256,uint256) setPaused(bool) timelockDelay() windowOf(address)");
    }

    function test_theEscrowExposesExactlyWhatItMeansTo() public view {
        _assertExactly("FeeEscrow.sol/FeeEscrow.json", "CHANGE_DELAY() FAST_DEADLINE() acceptBridge(address) accountedTotal() arbSys() bridgeReadyAt(address) bridgeReadyAtBlock(address) coinOf(uint256) confirm(uint256,uint256) contributedBy(address) deposit(address) depositFor(address,address) fastOpen() fastOverdue() flagOverdue(uint256) homeVault() homeVaultAlias() isBridge(address) keeper() l2RecipientOf(uint256) launcher() listingId() minRelease() owner() parentBlockSeconds() proposeBridge(address) recipientOf(address) release(address) releaseFast(address,address,bytes) removeBridge(address) reward() ticket(uint256) ticketCount() unspentOf(address)");
    }

    // ---- the 4663 side of a home-chain collection (docs/v6/home-chain-vault.md §10) ----

    /// The adapter: an ERC-20 1:1 against twin (`deposit`/`withdraw`, its own units, never the caller's twin without a deposit), the
    /// IWrappedCollection surface, and the intent's four steps. `withdraw` is the unwrap of the caller's OWN units and is meant.
    function test_theAdapterExposesExactlyWhatItMeansTo() public view {
        _assertExactly("HomeWrappedAdapter.sol/HomeWrappedAdapter.json", "ACK_DEADLINE() ONE_ITEM() acknowledge(uint256) allowance(address,address) approve(address,uint256) arbSys() balanceOf(address) beacon() custodian() decimals() deposit(uint256,address) dispatch(uint256) escrowed() held() homeDesk() homeDeskAlias() homeWrapper() l2Router() name() openTickets() parentBlockSeconds() redeemToBurn() redeemToRelist() refund(uint256) refunded(uint256) stray() symbol() ticket(uint256) ticketCount() totalSupply() transfer(address,uint256) transferFrom(address,address,uint256) withdraw(uint256,address) wx()");
    }

    /// The desk: an intent in (from the Outbox only), an acknowledgement down, a completion at the wrapper; no way out for its wX.
    function test_theDeskExposesExactlyWhatItMeansTo() public view {
        _assertExactly("HomeRedeemDesk.sol/HomeRedeemDesk.json", "L2_GAS_LIMIT() L2_MAX_FEE() ONE_ITEM() acknowledge(uint256) adapter() complete(uint256) completed() inbox() intent(uint256,uint8,address) intentOf(uint256) outbox() received() wrapper()");
    }

    /// The listing: the registry's roles borrowed, the owner's list and pause, the views.
    function test_theListingExposesExactlyWhatItMeansTo() public view {
        _assertExactly("HomeListing.sol/HomeListing.json", "count() guardian() isPaused(uint256) keeper() keeperPaused() list((uint64,address,uint8,string,address,address,address,address,address,address,address)) listed(bytes32) listing(uint256) listingRegistry() owner() ref(uint256) setPaused(uint256,bool) treasury() vaultOf(uint256)");
    }

    /// The home sweep vault is the sweep vault, exactly (test/security/SweepSecurity.t.sol holds that list), plus the escrow, the
    /// forward, and the three answers the escrow asks of its launcher.
    function test_theHomeSweepVaultIsTheSweepVaultPlusTheForward() public view {
        string[] memory base = _surface("SweepVault.sol/SweepVault.json");
        string[] memory extra = vm.split("escrow() forward(address) routerOf(address) creatorOf(address) listingOf(address)", " ");
        string memory all;
        for (uint256 i = 0; i < base.length; ++i) all = string.concat(all, i == 0 ? "" : " ", base[i]);
        for (uint256 i = 0; i < extra.length; ++i) all = string.concat(all, " ", extra[i]);
        _assertExactly("HomeSweepVault.sol/HomeSweepVault.json", all);
    }

    function test_theReceiverExposesExactlyWhatItMeansTo() public view {
        _assertExactly("HomeReceiver.sol/HomeReceiver.json", "DEAD() MAX_SLIPPAGE_BPS() absorb(uint256) buyback(address) claimWrapped(uint256) cooldown() creditProceeds(uint256) creditWrapped(uint256,uint256) escrow() homeVaultAlias() lastBuybackAt(address) maxTranche() proceedsOf(address) proceedsTotal() quote() reward() router() wrappedOwed(uint256) wrappedOwedTotal() wx()");
    }

    /// No way out by name, and no signature: a vault that could sign would let Seaport move its tokens under an order it never
    /// validated; a wrapper with a mint on somebody's word would be the canister again.
    function test_noContractHasAWayOutByName() public view {
        string[10] memory artifacts = ["HomeVaultRaffle.sol/HomeVaultRaffle.json", "HomeVaultBurn.sol/HomeVaultBurn.json",
            "HomeVaultYoyo.sol/HomeVaultYoyo.json", "HomeVaultWrap.sol/HomeVaultWrap.json", "HomeWrapper.sol/HomeWrapper.json",
            "HomeGuard.sol/HomeGuard.json", "FeeEscrow.sol/FeeEscrow.json", "HomeReceiver.sol/HomeReceiver.json",
            "HomeRedeemDesk.sol/HomeRedeemDesk.json", "HomeListing.sol/HomeListing.json"];
        string[14] memory forbidden = ["withdraw", "rescue", "drain", "sweep", "execute", "isValidSignature", "setReward", "setMarkup",
            "setPriceMultiplier", "setCollection", "setPolicy", "mint", "setApproval", "transferEther"];
        for (uint256 a = 0; a < artifacts.length; ++a) {
            string[] memory actual = _surface(artifacts[a]);
            for (uint256 i = 0; i < actual.length; ++i) {
                for (uint256 f = 0; f < forbidden.length; ++f) {
                    assertFalse(_startsWith(actual[i], forbidden[f]), string.concat(artifacts[a], " exposes ", actual[i]));
                }
            }
        }
    }

    /// The vaults' only ERC-721 movers are the three deliveries: a proven win, a burn, a wrap. No `transfer*` of a token id by a caller.
    function test_noVaultTakesADestinationFromItsCaller() public view {
        string[4] memory artifacts = ["HomeVaultRaffle.sol/HomeVaultRaffle.json", "HomeVaultBurn.sol/HomeVaultBurn.json",
            "HomeVaultYoyo.sol/HomeVaultYoyo.json", "HomeVaultWrap.sol/HomeVaultWrap.json"];
        for (uint256 a = 0; a < artifacts.length; ++a) {
            string[] memory actual = _surface(artifacts[a]);
            for (uint256 i = 0; i < actual.length; ++i) {
                assertFalse(_startsWith(actual[i], "transfer"), string.concat(artifacts[a], " exposes ", actual[i]));
                assertFalse(_startsWith(actual[i], "safeTransfer"), string.concat(artifacts[a], " exposes ", actual[i]));
                assertFalse(_startsWith(actual[i], "approve"), string.concat(artifacts[a], " exposes ", actual[i]));
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
