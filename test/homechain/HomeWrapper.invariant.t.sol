// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {HomeBase} from "./HomeBase.sol";
import {Clock} from "../utils/Clock.sol";
import {HomeWrapper} from "../../src/homechain/HomeWrapper.sol";
import {HomeGuard} from "../../src/homechain/HomeGuard.sol";
import {MockNft} from "../nft/NftMocks.sol";

/// Holders wrap, move, redeem and burn; anyone resolves, delivers, refunds and redraws; time passes; the guard pauses and is
/// unpaused. The ledger and the backing must hold at every step, and the backing is now a fact on the same chain.
contract HomeWrapHandler is Test {
    HomeWrapper public w;
    MockNft nft;
    HomeGuard guard;
    address timelock;
    address[3] actors;
    uint256 nextId = 1;
    uint256 public panics;
    uint256 public wraps;
    uint256 public redeems;
    uint256 public resolves;
    uint256 public refunds;

    constructor(HomeWrapper w_, MockNft nft_, HomeGuard guard_, address timelock_, address[3] memory actors_) {
        w = w_; nft = nft_; guard = guard_; timelock = timelock_; actors = actors_;
    }

    function _isPanic(bytes memory r) private pure returns (bool) {
        return r.length >= 4 && r[0] == 0x4e && r[1] == 0x48 && r[2] == 0x7b && r[3] == 0x71;
    }
    function _caught(bytes memory r) private { if (_isPanic(r)) panics++; }

    function wrap(uint8 who) external {
        address a = actors[who % 3];
        uint256 id = nextId++;
        nft.mint(a, id);
        vm.startPrank(a);
        nft.approve(address(w), id);
        try w.wrap(id, a) { wraps++; } catch (bytes memory r) { _caught(r); }
        vm.stopPrank();
    }

    function redeem(uint8 who, uint8 to, bool burn) external {
        address a = actors[who % 3];
        if (w.balanceOf(a) < 1e18) return;
        vm.prank(a);
        if (burn) { try w.redeemToBurn() { redeems++; } catch (bytes memory r) { _caught(r); } }
        else { try w.redeem(actors[to % 3]) { redeems++; } catch (bytes memory r) { _caught(r); } }
    }

    function move(uint8 from, uint8 to) external {
        address a = actors[from % 3];
        uint256 b = w.balanceOf(a);
        if (b == 0) return;
        vm.prank(a);
        w.transfer(actors[to % 3], b / 2);
    }

    function resolve(bytes32 r) external {
        uint256 id = w.nextToResolve();
        if (id >= w.ticketCount()) return;
        try w.resolve(id, abi.encodePacked(r == bytes32(0) ? bytes32(uint256(1)) : r)) { resolves++; } catch (bytes memory e) { _caught(e); }
    }

    function refund(uint8 which) external {
        uint256 n = w.ticketCount();
        if (n == 0) return;
        try w.refund(which % n) { refunds++; } catch (bytes memory e) { _caught(e); }
    }

    function redraw(uint8 which) external {
        uint256 n = w.ticketCount();
        if (n == 0) return;
        try w.redraw(which % n) {} catch (bytes memory e) { _caught(e); }
    }

    function unpause() external {
        if (guard.paused()) { vm.prank(timelock); guard.setPaused(false); }
    }

    function tick(uint32 secs) external {
        Clock.skip(bound(secs, 1, 8 days));
    }
}

contract HomeWrapperInvariantTest is HomeBase {
    HomeWrapHandler h;

    function setUp() public override {
        super.setUp();
        address[3] memory actors = [alice, bob, carol];
        h = new HomeWrapHandler(wNft, nft, guard, timelock, actors);
        targetContract(address(h));
    }

    /// Each held NFT backs one whole token or one open ticket, never both.
    function invariant_ledger() public view {
        assertEq(wNft.totalSupply() + wNft.openTickets() * 1e18, wNft.held() * 1e18);
    }

    /// The backing is on this chain: the wrapper owns exactly `held` NFTs, and every held item is owned by it.
    function invariant_backing() public view {
        assertEq(nft.balanceOf(address(wNft)), wNft.held());
        for (uint256 i = 0; i < wNft.itemCount(); ++i) {
            uint256 tokenId = wNft.item(i).tokenId;
            if (wNft.itemOf(tokenId) == i + 1) assertEq(nft.ownerOf(tokenId), address(wNft));
        }
    }

    /// Every held item is in exactly one place: free, pending, or on one resolved ticket.
    function invariant_everyHeldItemInOnePlace() public view {
        uint256 resolvedOpen;
        for (uint256 t = 0; t < wNft.ticketCount(); ++t) {
            if (wNft.ticket(t).stage == HomeWrapper.Stage.Resolved) resolvedOpen++;
        }
        assertEq(wNft.freeCount() + wNft.pendingCount() + resolvedOpen, wNft.held());
    }

    /// Tickets resolve strictly in order: nothing past `nextToResolve` is beyond Pinned.
    function invariant_strictOrder() public view {
        for (uint256 t = wNft.nextToResolve(); t < wNft.ticketCount(); ++t) {
            assertEq(uint8(wNft.ticket(t).stage), uint8(HomeWrapper.Stage.Pinned));
        }
    }

    /// The guard counts what the wrapper holds (the wrapper is its only registered holder of `nft` in this campaign).
    function invariant_guardCountsTheWrapper() public view {
        assertEq(guard.capOf(address(nft)).items, wNft.held());
    }

    function invariant_noPanics() public view {
        assertEq(h.panics(), 0);
    }
}
