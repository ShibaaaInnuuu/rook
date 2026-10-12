// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {stdJson} from "forge-std/StdJson.sol";
import {WrapBase} from "../utils/WrapBase.sol";
import {Clock} from "../utils/Clock.sol";
import {Wrapper} from "../../src/Wrapper.sol";
import {Distributor} from "../../src/Distributor.sol";

/// The attacks on the wrapper that the design document did not close as written, each shown closed.
///
/// @dev docs/v3/wrapper.md said a redemption "picks a frog from the vault's currently held set". Read literally, that set can
/// be changed AFTER the round that picks from it is public — by resolving other tickets first, by refunding an expired ticket,
/// or by wrapping a new frog — and each change moves which frog a known random number lands on. Whoever can make one of
/// those changes can choose between outcomes: grind the draw. The contract fixes the set each ticket draws from before its
/// round exists (strict order, and a stamp on every frog saying when it became free; "Built", 4). These tests run the same
/// ticket in two worlds that differ only by the attacker's move after the round is known, and require the same frog.
contract WrapperSecurityTest is WrapBase {
    using stdJson for string;

    address alice = makeAddr("alice");
    address bob = makeAddr("bob");

    /// W1. A creator who sees the round cannot re-roll it by wrapping another frog before anyone resolves.
    function testFuzz_W1_wrappingAfterTheRoundIsPublicCannotMoveTheDraw(bytes32 r) public {
        _frog(0); _frog(1);
        _give(alice, 1);
        uint256 id = _redeem(alice);
        uint256 aid2 = _receipt(2);                // the attacker's frog, ready to wrap
        vm.prank(creatorA);
        distributor.choose(0, aid2, Distributor.Mode.Never, address(0));
        if (r == bytes32(0)) r = bytes32(uint256(7));
        beacon.prove(wrapper.ticket(id).round, abi.encodePacked(r));   // the round is public

        uint256 snap = vm.snapshotState();
        _resolve(id, r);
        uint256 honest = wrapper.ticket(id).acquisitionId;
        vm.revertToState(snap);
        vm.prank(creatorA);
        wrapper.wrap(aid2);                        // the attacker's move
        _resolve(id, r);
        assertEq(wrapper.ticket(id).acquisitionId, honest, "wrapping after the reveal changed the frog");
    }

    /// W2. Whoever can refund an expired ticket cannot time the refund to steer a later ticket's draw.
    function testFuzz_W2_refundTimingCannotMoveTheDraw(bytes32 r) public {
        _frog(0); _frog(1); _frog(2);
        _give(alice, 1);
        _give(bob, 1);
        uint256 a = _redeem(alice);
        _resolve(a, keccak256("a"));
        Clock.skip(7 days + 72 hours);             // a is refundable from here (the deadline and the refund grace)
        uint256 b = _redeem(bob);
        if (r == bytes32(0)) r = bytes32(uint256(7));
        beacon.prove(wrapper.ticket(b).round, abi.encodePacked(r));

        uint256 snap = vm.snapshotState();
        _resolve(b, r);
        uint256 resolvedFirst = wrapper.ticket(b).acquisitionId;
        vm.revertToState(snap);
        vm.prank(custodian);
        wrapper.markUndeliverable(a);              // the custodian's attestation (review2 R3 §4)
        wrapper.refund(a);                         // the attacker's move: refund first, putting a frog back
        _resolve(b, r);
        assertEq(wrapper.ticket(b).acquisitionId, resolvedFirst, "refunding before resolving changed the frog");
    }

    /// W3. Nobody can resolve a later ticket first to change what an earlier one draws from.
    function test_W3_resolutionOrderIsNotAChoice() public {
        _frog(0); _frog(1); _frog(2);
        _give(alice, 2);
        _redeem(alice);
        uint256 later = _redeem(alice);
        vm.expectRevert(abi.encodeWithSelector(Wrapper.OutOfOrder.selector, 0));
        wrapper.resolve(later, abi.encodePacked(keccak256("r")));
    }

    /// W4. Without the stamp, W1 would have worked: the same random number over the set with and without the late frog lands
    /// on different frogs for some rounds. This finds one, so W1 is testing something real and not a coincidence.
    function test_W4_theAttackW1ClosesIsReal() public view {
        bool found;
        for (uint256 k = 0; k < 64 && !found; ++k) {
            uint256 x = uint256(keccak256(abi.encode(keccak256(abi.encode(k)), address(wrapper), uint256(0))));
            found = (x % 2) != (x % 3);            // two free frogs, or three once the attacker wraps one
        }
        assertTrue(found);
    }

    // ---- the sequencer's clock: src/lib/Wait.sol ----

    /// S1. A sequencer stamping blocks up to an hour ahead cannot open a refund before the deadline has passed on the parent
    /// chain's clock too — refunding early would race a delivery the custodian is still allowed to send.
    function test_S1_aTimestampJumpAloneCannotOpenARefundEarly() public {
        _frog(0);
        _give(alice, 1);
        uint256 id = _redeem(alice);
        _resolve(id, keccak256("r"));
        vm.prank(custodian);
        wrapper.markUndeliverable(id);                     // the custodian's attestation (review2 R3 §4)
        Clock.skipTimeOnly(7 days + 72 hours + 1 hours);   // past the deadline and the refund grace, by the sequencer alone
        vm.expectRevert(Wrapper.TooEarly.selector);
        wrapper.refund(id);
        vm.roll(vm.getBlockNumber() + (7 days + 72 hours) / 12 + 1);   // the parent chain catches up
        wrapper.refund(id);
    }

    /// S2. Nobody can re-pin a round at all (C-02, E-03: the pinned round is final), and a timestamp jump alone cannot bring the
    /// pinned-ticket refund forward.
    function test_S2_noRepin_andATimestampJumpAloneCannotRefundAPinnedTicketEarly() public {
        _frog(0);
        _give(alice, 1);
        uint256 id = _redeem(alice);
        (bool ok,) = address(wrapper).call(abi.encodeWithSignature("redraw(uint256)", id));
        assertFalse(ok, "there is no redraw");
        Clock.skipTimeOnly(7 days + 1 hours);
        vm.expectRevert(Wrapper.TooEarly.selector);
        wrapper.refund(id);
    }

    // ---- no way to mint, take or stop ----

    /// The wrapper's whole ABI, exactly: nothing mints but `wrap`, `wrapDeposit` (the custodian only) and `refund`, nothing takes a frog or a token from anybody,
    /// and there is no owner, pause, setter or sweep. A function added later fails here until somebody decides who may call it.
    function test_theWrapperExposesExactlyWhatItMeansTo() public view {
        string[81] memory list = [
            // the ERC-20
            "name()", "symbol()", "decimals()", "totalSupply()", "balanceOf(address)", "transfer(address,uint256)",
            "allowance(address,address)", "approve(address,uint256)", "transferFrom(address,address,uint256)",
            // the life of a frog and a ticket
            "wrap(uint256)", "redeem(string)", "redeemToBurn()", "redeemToRelist()", "refunded(uint256)", "BURN_ADDRESS()", "resolve(uint256,bytes)", "recordDelivery(uint256,bytes32)",
            "refund(uint256)",
            // canister review2 N-2 (fixes-R3-contracts.md §4): the custodian's word that a delivery can no longer confirm
            "markUndeliverable(uint256)", "undeliverable(uint256)", "LATE_DELIVERY_WINDOW()",
            // a holder's deposit (docs/v3/sweep.md, 1a): the custodian only
            "wrapDeposit(string,bytes32[],bytes32,uint32,address)", "DEPOSIT_FLAG()", "depositCount()", "depositMinted(bytes32)",
            "inscriptionHeld(bytes32)", "leafOfFrog(uint256)",
            // what it was built with, and constants
            "registry()", "distributor()", "beacon()", "listingId()", "distribution()", "custodian()", "custodyAddress()", "ONE_ITEM()", "PIN_AHEAD()",
            "PIN_TIMEOUT()", "DELIVERY_DEADLINE()", "REFUND_GRACE()", "parentBlockSeconds()",
            // the books
            "wrapped(uint256)", "held()", "openTickets()", "nextToResolve()", "pendingHead()", "ticket(uint256)",
            "ticketCount()", "freeCount()", "freeAt(uint256)", "pendingCount()", "pendingAt(uint256)",
            // the floor module (docs/v5/floor-module.md §3): wired once by the deployer before anything exists; only it mints
            // ETH-backed units, burns its own, and admits and returns rented frogs; only the custodian records a return
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
        string memory json = vm.readFile("out/Wrapper.sol/Wrapper.json");
        string[] memory actual = vm.parseJsonKeys(json, ".methodIdentifiers");
        uint256 expected = list.length;
        assertEq(actual.length, expected, "the wrapper exposes a function that is not on the list, or lost one that is");
        for (uint256 j = 0; j < expected; ++j) {
            bool found;
            for (uint256 i = 0; i < actual.length; ++i) {
                if (keccak256(bytes(actual[i])) == keccak256(bytes(list[j]))) { found = true; break; }
            }
            assertTrue(found, string.concat("missing: ", list[j]));
        }
    }
}
