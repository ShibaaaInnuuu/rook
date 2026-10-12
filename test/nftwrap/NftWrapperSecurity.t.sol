// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {NftWrapBase} from "./NftWrapBase.sol";
import {Clock} from "../utils/Clock.sol";
import {NftWrapper} from "../../src/nft/NftWrapper.sol";
import {IWrappedCollection} from "../../src/interfaces/IWrappedCollection.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

/// test/security/WrapperSecurity.t.sol, for the NFT wrapper. The concern docs/v4/nft.md §8 raised about any NFT wrapper — that
/// redeemers take the best ids first — is closed only if nobody can choose or steer which id a burn gets. Each test runs one
/// ticket in two worlds that differ only by an attacker's move after the round is known, and requires the same NFT.
contract NftWrapperSecurityTest is NftWrapBase {
    /// N1. The canister, seeing the round, cannot re-roll it by minting another NFT before anyone resolves.
    function testFuzz_N1_mintingAfterTheRoundIsPublicCannotMoveTheDraw(bytes32 r) public {
        _mint(1, alice); _mint(2, alice);
        uint256 id = _redeem(alice);
        if (r == bytes32(0)) r = bytes32(uint256(7));
        beacon.prove(w.ticket(id).round, abi.encodePacked(r));   // the round is public

        uint256 snap = vm.snapshotState();
        _resolve(id, r);
        uint256 honest = _tokenOf(id);
        vm.revertToState(snap);
        _mint(3, curve);                                          // the attacker's move: a purchase
        vm.prank(custodian);
        w.mintDeposit(4, keccak256("a deposit"), curve);          // and a deposit
        _resolve(id, r);
        assertEq(_tokenOf(id), honest, "minting after the reveal changed the NFT");
    }

    /// N2. Whoever can refund an expired ticket cannot time the refund to steer a later ticket's draw.
    function testFuzz_N2_refundTimingCannotMoveTheDraw(bytes32 r) public {
        _mint(1, alice); _mint(2, alice); _mint(3, bob);
        uint256 a = _redeem(alice);
        _resolve(a, keccak256("a"));
        Clock.skip(7 days);                                       // a is refundable from here
        uint256 b = _redeem(bob);
        if (r == bytes32(0)) r = bytes32(uint256(7));
        beacon.prove(w.ticket(b).round, abi.encodePacked(r));

        uint256 snap = vm.snapshotState();
        _resolve(b, r);
        uint256 resolvedFirst = _tokenOf(b);
        vm.revertToState(snap);
        w.refund(a);                                              // the attacker's move: put an NFT back first
        _resolve(b, r);
        assertEq(_tokenOf(b), resolvedFirst, "refunding before resolving changed the NFT");
    }

    /// N3. Nobody can resolve a later ticket first to change what an earlier one draws from.
    function test_N3_resolutionOrderIsNotAChoice() public {
        _mint(1, alice); _mint(2, alice); _mint(3, alice);
        _redeem(alice);
        uint256 later = _redeem(alice);
        vm.expectRevert(abi.encodeWithSelector(NftWrapper.OutOfOrder.selector, 0));
        w.resolve(later, abi.encodePacked(keccak256("r")));
    }

    /// N4. Without the stamp, N1 would have worked: for some rounds the same random number lands on different NFTs over two
    /// and over three free ones. This finds one, so N1 tests something real.
    function test_N4_theAttackN1ClosesIsReal() public view {
        bool found;
        for (uint256 k = 0; k < 64 && !found; ++k) {
            uint256 x = uint256(keccak256(abi.encode(keccak256(abi.encode(k)), address(w), uint256(0))));
            found = (x % 2) != (x % 3);
        }
        assertTrue(found);
    }

    /// N5. Who resolves, and with what bytes once the round is proven, changes nothing.
    function testFuzz_N5_theResolverCannotChooseEither(bytes32 r, address resolver, bytes calldata junk) public {
        for (uint256 i = 1; i <= 6; ++i) _mint(i, alice);
        uint256 id = _redeem(alice);
        if (r == bytes32(0)) r = bytes32(uint256(7));
        beacon.prove(w.ticket(id).round, abi.encodePacked(r));

        uint256 snap = vm.snapshotState();
        _resolve(id, r);
        uint256 honest = _tokenOf(id);
        vm.revertToState(snap);
        vm.prank(resolver);
        w.resolve(id, junk);                                      // the round is already proven: the signature is not read
        assertEq(_tokenOf(id), honest);
    }

    /// N6. A redeemer cannot pick: the burn takes no id, the round does not exist yet, and nothing about the ticket can be
    /// changed afterwards (there is no function that takes a ticket and a destination, or a ticket and an id).
    function test_N6_aRedeemerChoosesNothingButWhereItGoes() public {
        _mint(1, alice); _mint(2, alice);
        uint256 id = _redeem(alice);
        assertEq(beacon.randomnessOf(w.ticket(id).round), bytes32(0), "the deciding round is in the future at the burn");
        assertGt(beacon.timeOf(w.ticket(id).round), block.timestamp);
        string[] memory fns = vm.parseJsonKeys(vm.readFile("out/NftWrapper.sol/NftWrapper.json"), ".methodIdentifiers");
        for (uint256 i = 0; i < fns.length; ++i) {
            bytes32 h = keccak256(bytes(fns[i]));
            // the only state-changing entry points a redeemer has: three burns (to a person, to DEAD, to a listing), and the permissionless steps
            if (h == keccak256("redeem(address)") || h == keccak256("redeemToBurn()") || h == keccak256("redeemToRelist()")) continue;
            assertTrue(h != keccak256("redeem(uint256)") && h != keccak256("redeem(uint256,address)")
                && h != keccak256("setDestination(uint256,address)"), fns[i]);
        }
    }

    /// N7. A refund is not a re-roll a redeemer can trigger: only the custodian's silence opens it, and only after 7 days on
    /// both clocks. (The canister delivers with `transferFrom`, which a recipient cannot refuse: docs/v4/nft-wrapper.md §5.3.)
    function test_N7_theRedeemerCannotRefundADeliverableTicket() public {
        _mint(1, alice); _mint(2, alice);
        uint256 id = _redeem(alice);
        _resolve(id, keccak256("r"));
        vm.prank(alice);
        vm.expectRevert(NftWrapper.TooEarly.selector);
        w.refund(id);
        Clock.skip(6 days);
        vm.prank(alice);
        vm.expectRevert(NftWrapper.TooEarly.selector);
        w.refund(id);
        _deliver(id);
        Clock.skip(2 days);
        vm.prank(alice);
        vm.expectRevert(NftWrapper.WrongStage.selector);
        w.refund(id);
    }

    /// N8. A ticket's NFT is promised once: while a ticket names it, no other ticket can draw it, and it cannot be minted again.
    function test_N8_anNftOnATicketIsNeitherDrawnAgainNorMintedAgain() public {
        _mint(1, alice);
        uint256 a = _redeem(alice);
        _resolve(a, keccak256("r"));
        vm.prank(custodian);
        vm.expectRevert(NftWrapper.AlreadyHeld.selector);
        w.mint(1, keccak256("pretend it came back"), bob);
        _mint(2, bob);
        uint256 b = _redeem(bob);
        _resolve(b, keccak256("s"));
        assertEq(_tokenOf(b), 2);
    }

    // ---- the sequencer's clock: src/lib/Wait.sol ----

    /// S1. A sequencer stamping blocks an hour ahead cannot open a refund before the parent chain's clock agrees.
    function test_S1_aTimestampJumpAloneCannotOpenARefundEarly() public {
        _mint(1, alice);
        uint256 id = _redeem(alice);
        _resolve(id, keccak256("r"));
        Clock.skipTimeOnly(7 days + 1 hours);
        vm.expectRevert(NftWrapper.TooEarly.selector);
        w.refund(id);
        vm.roll(block.number + 7 days / 12 + 1);
        w.refund(id);
    }

    /// S2. Nor re-pin a round early, which would let whoever calls at the right moment choose between two rounds.
    function test_S2_aTimestampJumpAloneCannotRepinEarly() public {
        _mint(1, alice);
        uint256 id = _redeem(alice);
        Clock.skipTimeOnly(24 hours + 1 hours);
        vm.expectRevert(NftWrapper.TooEarly.selector);
        w.redraw(id);
        vm.roll(block.number + 24 hours / 12 + 1);
        w.redraw(id);
    }

    // ---- the shape consumers rely on ----

    /// The curve, desk, sweep vault and buy queue see this wrapper only through IWrappedCollection (docs/v4/nft-wrapper.md §3).
    function test_itIsAWrappedCollection() public {
        IWrappedCollection c = IWrappedCollection(address(w));
        _mint(1, curve);
        assertEq(c.ONE_ITEM(), 1e18);
        assertEq(c.custodian(), custodian);
        assertEq(c.held(), 1);
        assertEq(c.openTickets(), 0);
        assertEq(IERC20(address(c)).balanceOf(curve), 1e18);
        vm.prank(curve);
        uint256 id = c.redeemToBurn();
        assertEq(c.openTickets(), 1);
        assertFalse(c.refunded(id));
        _resolve(id, keccak256("r"));
        Clock.skip(7 days);
        w.refund(id);
        assertTrue(c.refunded(id));
        assertEq(IERC20(address(c)).balanceOf(curve), 1e18, "the burner has its unit back to retire again");
    }

    /// The interface is small and exact: a function added to it is a function every wrapper must now have.
    function test_theInterfaceIsExactly() public pure {
        bytes4 id = type(IWrappedCollection).interfaceId;
        bytes4 expected = IWrappedCollection.ONE_ITEM.selector ^ IWrappedCollection.custodian.selector
            ^ IWrappedCollection.held.selector ^ IWrappedCollection.openTickets.selector
            ^ IWrappedCollection.redeemToBurn.selector ^ IWrappedCollection.redeemToRelist.selector
            ^ IWrappedCollection.refunded.selector;
        assertEq(id, expected, "IWrappedCollection's own functions changed");
        assertEq(IWrappedCollection.redeemToBurn.selector, bytes4(keccak256("redeemToBurn()")));
        assertEq(IWrappedCollection.redeemToRelist.selector, bytes4(keccak256("redeemToRelist()")));
        assertEq(IWrappedCollection.refunded.selector, bytes4(keccak256("refunded(uint256)")));
    }

    // ---- no way to mint, take or stop ----

    /// The wrapper's whole ABI, exactly: nothing mints but `mint` / `mintDeposit` (the custodian only) and `refund`, nothing takes
    /// an NFT or a token from anybody, and there is no owner, pause, setter or sweep.
    function test_theWrapperExposesExactlyWhatItMeansTo() public view {
        string[74] memory list = [
            // the ERC-20
            "name()", "symbol()", "decimals()", "totalSupply()", "balanceOf(address)", "transfer(address,uint256)",
            "allowance(address,address)", "approve(address,uint256)", "transferFrom(address,address,uint256)",
            // minting: the custodian only
            "mint(uint256,bytes32,address)", "mintDeposit(uint256,bytes32,address)",
            // the life of a ticket
            "redeem(address)", "redeemToBurn()", "redeemToRelist()", "redraw(uint256)", "resolve(uint256,bytes)",
            "recordDelivery(uint256,bytes32)", "refund(uint256)",
            // what it was built with, and constants
            "beacon()", "chainId()", "collection()", "custodian()", "ONE_ITEM()", "PIN_AHEAD()", "REDRAW_AFTER()",
            "DELIVERY_DEADLINE()", "DEAD()", "DEPOSIT_MAGIC()",
            // the books
            "itemOf(uint256)", "sourceUsed(bytes32)", "held()", "openTickets()", "burned()", "nextToResolve()", "pendingHead()",
            "refunded(uint256)", "ticket(uint256)", "ticketCount()", "item(uint256)", "itemCount()", "freeCount()",
            "freeAt(uint256)", "pendingCount()", "pendingAt(uint256)",
            // the floor module (docs/v5/floor-module.md §3): wired once by the deployer before anything exists; only it mints
            // ETH-backed units, burns its own, and admits and returns rented NFTs; only the custodian records a return
            "deployer()", "floorModule()", "wireFloorModule(address)", "ethBacked()", "moduleMint(address,uint256)",
            "moduleBurn(uint256)", "admitRental(bytes,bytes)", "returnRental(uint256)", "processLeaving()",
            "recordReturn(uint256,bytes32)", "openReturns()", "rentedHeld()", "leavingHead()", "leavingCount()",
            "rentalState(uint256)", "rentalOf(uint256)", "rentalCount()", "rentalAt(uint256)",
            // H11 (docs/v5/custody-hardening.md): the custodian's daily mint rate; the deployer (the timelock) lowers it at once
            // or proposes a raise anyone may accept after seven days on both clocks
            "MINT_WINDOW()", "DEFAULT_MINTS_PER_DAY()", "LIMIT_DELAY()", "maxMintsPerDay()", "itemMints()", "pendingMintLimit()",
            "pendingLimitAt()", "pendingLimitAtBlock()", "initMintLimit(uint256)", "proposeMintLimit(uint256)", "acceptMintLimit()",
            "mintRoom()"
        ];
        string[] memory actual = vm.parseJsonKeys(vm.readFile("out/NftWrapper.sol/NftWrapper.json"), ".methodIdentifiers");
        assertEq(actual.length, list.length, "the wrapper exposes a function that is not on the list, or lost one that is");
        for (uint256 j = 0; j < list.length; ++j) assertTrue(_has(actual, list[j]), string.concat("missing: ", list[j]));
    }

    function test_noWayOutByName() public view {
        string[] memory actual = vm.parseJsonKeys(vm.readFile("out/NftWrapper.sol/NftWrapper.json"), ".methodIdentifiers");
        string[10] memory forbidden = ["withdraw", "rescue", "drain", "release", "sweep", "pause", "setCustodian", "setCollection",
            "owner", "execute"];
        for (uint256 i = 0; i < actual.length; ++i) {
            for (uint256 f = 0; f < forbidden.length; ++f) assertFalse(_startsWith(actual[i], forbidden[f]), actual[i]);
        }
    }

    function _has(string[] memory xs, string memory x) private pure returns (bool) {
        for (uint256 i = 0; i < xs.length; ++i) if (keccak256(bytes(xs[i])) == keccak256(bytes(x))) return true;
        return false;
    }

    function _startsWith(string memory s, string memory prefix) private pure returns (bool) {
        bytes memory b = bytes(s);
        bytes memory p = bytes(prefix);
        if (p.length > b.length) return false;
        for (uint256 i = 0; i < p.length; ++i) if (b[i] != p[i]) return false;
        return true;
    }
}
