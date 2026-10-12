// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

// Confirmation review of DF-M-1 (deep-fuzz-review.md). Copy to test/ and run:
//   forge test --match-path test/DfReviewNft.t.sol -vv
// The NftWrapper (chain-1 collection, rentals ON in code) with a real FloorModule: the first end-to-end rental round trip on
// that wrapper in the repo (before this, only the frog Wrapper's rentals ran against a module).

import {Base} from "./utils/Base.sol";
import {Merkle} from "./utils/Merkle.sol";
import {FloorModule} from "../src/floor/FloorModule.sol";
import {PosterSource} from "../src/floor/PosterSource.sol";
import {NftWrapper} from "../src/nft/NftWrapper.sol";
import {IKeeperSource} from "../src/interfaces/IKeeperSource.sol";
import {IFloorWrapped} from "../src/interfaces/IFloorWrapped.sol";
import {IRandomnessBeacon} from "../src/interfaces/IRandomnessBeacon.sol";

contract DfReviewNft is Base {
    NftWrapper w;
    PosterSource poster;
    FloorModule module;
    address collection = makeAddr("BAYC on chain 1");
    address payee = makeAddr("payee");
    address platform = makeAddr("platform");
    address alice = makeAddr("alice");
    address renter = makeAddr("renter");
    address home = makeAddr("renter's return address on chain 1");
    uint256 constant ONE = 1e18;
    uint256 constant F = 0.93 ether;
    bytes32[] leaves;

    function _params() internal pure returns (FloorModule.Params memory p) {
        p.baseSpreadBps = 15; p.volWeightBps = 10_000; p.staleSpreadBps = 100; p.stressSpreadBps = 300; p.maxSpreadBps = 1_000;
        p.maxMoveBps = 500; p.convBps = 300; p.bufferBps = 500; p.mintPerHour = 5e18; p.burnPerHour = 5e18;
        p.maxPublicEthBacked = 10e18; p.maxEthBacked = 40e18; p.revenueTarget = 1 ether; p.rentTerm = 30 days; p.premiumBps = 1_000;
        p.rentShareBps = 5_000; p.maxRentals = 20; p.lowConfidenceSpreadBps = 100; p.fullConfidenceBps = 8_000;
        p.minConfidenceBps = 3_000; p.maxItemsPerDay = 40; p.reserveBps = 2_500; p.fillBandBps = 300;
    }

    function setUp() public override {
        super.setUp();
        w = new NftWrapper(IRandomnessBeacon(address(beacon)), 1, collection, custodian, "Wrapped Ape", "wAPE");
        w.initMintLimit(1_000);
        poster = new PosterSource(IKeeperSource(address(registry)), custodian);
        module = new FloorModule(IFloorWrapped(address(w)), IKeeperSource(address(poster)), payee, platform, address(0), _params(), PBS);
        w.wireFloorModule(address(module));
        for (uint256 t = 0; t < 8; ++t) leaves.push(keccak256(abi.encode(keccak256(abi.encode(t)))));
        vm.startPrank(custodian);
        module.postRentalRoot(Merkle.root(leaves));
        module.postFloor(F, 10_000, 50);
        vm.stopPrank();
        vm.deal(alice, 10 ether);
        vm.prank(alice);
        module.mint{value: 3 ether}(alice, 0);
    }

    function _skip(uint256 secs) internal { vm.warp(vm.getBlockTimestamp() + secs); vm.roll(vm.getBlockNumber() + secs / PBS + 1); }
    function _post() internal { vm.prank(custodian); module.postFloor(F, 10_000, 50); }

    function _rent(uint256 tokenId, uint256 n) internal returns (uint256 rid, uint256 itemId) {
        vm.prank(custodian);
        rid = module.rentIn(renter, abi.encode(home), Merkle.proof(leaves, tokenId),
            abi.encode(tokenId, keccak256(abi.encode("rent deposit", tokenId, n))));
        itemId = module.rental(rid).itemId;
    }

    function _endAndReturn(uint256 rid, uint256 itemId, uint256 n) internal {
        _skip(31 days);
        _post();
        module.endRental(rid);
        assertEq(uint256(w.rentalState(itemId)), uint256(IFloorWrapped.RentalState.Ready), "no ticket waiting: Ready at once");
        vm.prank(custodian);
        w.recordReturn(itemId, keccak256(abi.encode("return tx", n)));
    }

    function _books() internal view {
        assertEq(w.totalSupply() + w.openTickets() * ONE, w.held() * ONE + w.ethBacked(), "(W1) supply + open == held x ONE + ethBacked");
        assertGe(address(module).balance, module.backing() + module.reserve() + module.surplus() + module.revenue() + module.gradEth()
            + module.rentalEscrow() + module.renterPool() + module.owedTotal(), "solvent");
        if (w.ethBacked() == 0) assertEq(module.backing(), 0, "no backing without units");
    }

    /// Rent → end → return on the NftWrapper: the rental closes in recordReturn, backing per unit is restored exactly.
    function test_nft_roundTrip_closesInRecordReturn() public {
        uint256 b0 = module.backing();
        uint256 e0 = w.ethBacked();
        (uint256 rid, uint256 itemId) = _rent(3, 0);
        uint256 escrow = module.rental(rid).escrow;
        assertEq(module.backing() + escrow, b0);
        assertEq(w.ethBacked(), e0 - ONE);
        _endAndReturn(rid, itemId, 0);
        assertEq(uint256(module.rental(rid).status), uint256(FloorModule.RentStatus.Closed), "closed by the return itself");
        assertEq(module.backing(), b0, "backing restored");
        assertEq(w.ethBacked(), e0, "units restored");
        assertEq(module.rentalEscrow(), 0);
        assertEq(module.leavingRentals(), 0);
        vm.expectRevert(FloorModule.NothingToSettle.selector);
        module.settleRental(rid);
        _books();
    }

    /// The return never depends on the module's pause, the keeper, or a fresh floor: it succeeds with the keeper paused and the
    /// floor long stale.
    function test_nft_returnSucceedsWithKeeperPausedAndStaleFloor() public {
        (uint256 rid, uint256 itemId) = _rent(4, 0);
        _skip(31 days);
        _post();
        module.endRental(rid);
        _skip(10 days);                                       // the floor goes stale
        vm.prank(owner);
        registry.pauseKeeper();
        vm.prank(owner);
        poster.pause();
        assertTrue(poster.keeperPaused());
        assertFalse(module.fresh());
        vm.prank(custodian);
        w.recordReturn(itemId, keccak256("tx"));
        assertEq(uint256(module.rental(rid).status), uint256(FloorModule.RentStatus.Closed));
        _books();
    }

    /// The same token rented again after its return: a new itemId, a new rental; the mapping points each return at its own rental.
    function test_nft_reRentSameToken() public {
        (uint256 rid1, uint256 item1) = _rent(5, 1);
        _endAndReturn(rid1, item1, 1);
        (uint256 rid2, uint256 item2) = _rent(5, 2);
        assertTrue(item2 != item1, "a new item id per admission");
        assertEq(uint256(module.rental(rid2).status), uint256(FloorModule.RentStatus.Lent));
        _endAndReturn(rid2, item2, 2);
        assertEq(uint256(module.rental(rid1).status), uint256(FloorModule.RentStatus.Closed));
        assertEq(uint256(module.rental(rid2).status), uint256(FloorModule.RentStatus.Closed));
        assertEq(module.rentalEscrow(), 0);
        _books();
    }

    /// Unreachable-by-construction states, forced by pranking the wrapper: a Lent rental and an already-closed one both revert.
    function test_rentalReturned_wrongStatusReverts() public {
        (uint256 rid, uint256 itemId) = _rent(6, 0);
        vm.prank(address(w));
        vm.expectRevert(FloorModule.WrongStatus.selector);
        module.rentalReturned(itemId);                        // Lent, not Leaving
        _endAndReturn(rid, itemId, 0);
        vm.prank(address(w));
        vm.expectRevert(FloorModule.WrongStatus.selector);
        module.rentalReturned(itemId);                        // already closed: no double credit of the escrow
    }

    /// The window of DF-M-1 on the NftWrapper: a burn of every other unit, then the return, then burn the returned unit: it is paid
    /// its full backing and nothing is left behind.
    function test_nft_burnAfterReturnPaysFullBacking() public {
        (uint256 rid, uint256 itemId) = _rent(7, 0);
        _skip(31 days);
        _post();
        module.endRental(rid);
        uint256 bal = w.balanceOf(alice);
        vm.startPrank(alice);
        w.approve(address(module), type(uint256).max);
        // burn is rate-limited per hour; burn everything alice holds in chunks
        while (w.ethBacked() != 0) {                           // every ETH-backed unit (alice keeps the one the rented item backs)
            uint256 amt = w.ethBacked() > 4e18 ? 4e18 : w.ethBacked();
            module.burn(amt, 0, alice);
            vm.stopPrank(); _skip(2 hours); _post(); vm.startPrank(alice);
        }
        vm.stopPrank();
        assertEq(module.backing(), 0);
        assertGt(bal, 0);
        vm.prank(custodian);
        w.recordReturn(itemId, keccak256("tx"));
        assertEq(module.backing(), module.rental(rid).escrow, "the returned unit is backed by its escrow at once");
        _books();
        // and that unit burns for its full backing less the spread: nothing stranded, nothing short
        _skip(2 hours); _post();
        vm.startPrank(alice);
        uint256 got = module.burn(ONE, 0, alice);
        vm.stopPrank();
        assertEq(module.backing(), 0, "nothing left behind");
        assertGt(got, module.rental(rid).escrow * 97 / 100);
        _books();
    }
}
