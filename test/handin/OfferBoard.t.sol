// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Clock} from "../utils/Clock.sol";
import {OfferBoard} from "../../src/handin/OfferBoard.sol";
import {IWrappedCollection} from "../../src/interfaces/IWrappedCollection.sol";

/// Stands in for the wrapper: the board reads nothing of it but `custodian()`.
contract MockCustodied {
    address public custodian;
    constructor(address c) { custodian = c; }
    function set(address c) external { custodian = c; }
}

/// A submitter that refuses ETH: a claim to it must still land (pull payments), only its withdraw fails.
contract RefusingSubmitter {
    OfferBoard b;
    constructor(OfferBoard b_) { b = b_; }
    function submit(bytes32 h, uint256 ask) external payable returns (uint256) { return b.submit{value: msg.value}(h, ask, "aaaa1111i0"); }
    function pull() external returns (uint256) { return b.withdraw(); }
}

/// docs/v6/offer-handins.md §2: the board's exact ABI, its money, its clocks, and who may do what.
contract OfferBoardTest is Test {
    uint256 constant PBS = 12;
    uint256 constant BOND = 0.01 ether;
    uint256 constant REWARD = 0.01 ether;
    uint256 constant SLASH_DELAY = 1 hours;
    uint256 constant EXPIRY = 24 hours;
    uint256 constant DAY_CAP = 0.2 ether;
    uint256 constant MAX_OPEN = 64;
    uint256 constant PER_ADDR = 4;

    address custodian = makeAddr("custodian (the canister's EVM address)");
    address bot = makeAddr("bot");
    address other = makeAddr("other bot");
    address funder = makeAddr("funder (the fee router)");
    MockCustodied wrapper;
    OfferBoard board;

    string constant ID = "6fb976ab49dcec017f1e201e84395983204ae1a7c2abf7ced0a85d692e442799i0";

    function setUp() public {
        vm.warp(1_800_000_000);
        vm.roll(26_000_000);
        wrapper = new MockCustodied(custodian);
        board = new OfferBoard(IWrappedCollection(address(wrapper)), BOND, REWARD, SLASH_DELAY, EXPIRY, DAY_CAP, MAX_OPEN, PER_ADDR, PBS);
        vm.deal(bot, 10 ether);
        vm.deal(other, 10 ether);
        vm.deal(funder, 10 ether);
    }

    function _submit(address who, bytes32 h, uint256 ask) internal returns (uint256 id) {
        vm.prank(who);
        id = board.submit{value: BOND}(h, ask, ID);
    }

    function _fund(uint256 amount) internal {
        vm.prank(funder);
        board.fund{value: amount}();
    }

    function _books() internal view {
        assertEq(address(board).balance, board.bonded() + board.rewardPool() + board.owedTotal(), "the books");
    }

    // ---- the exact ABI (what autobuyer/canister-rs/src/handin.rs pins) ------------------------------------------------------------

    function test_theSelectorsAreTheOnesTheCanisterPins() public pure {
        assertEq(OfferBoard.submit.selector, bytes4(0x7f3b74cc));
        assertEq(OfferBoard.claim.selector, bytes4(0x379607f5));
        assertEq(OfferBoard.slash.selector, bytes4(0x45bc4d10));
        assertEq(OfferBoard.refund.selector, bytes4(0x278ecde1));
        assertEq(OfferBoard.openIds.selector, bytes4(0xc3f95de6));
        assertEq(OfferBoard.openCount.selector, bytes4(0xcfaf380e));
        assertEq(OfferBoard.submissionOf.selector, bytes4(0x74505d71));
        assertEq(OfferBoard.params.selector, bytes4(0xcff0ab96));
        assertEq(bytes4(keccak256("wrapper()")), bytes4(0xac210cc7));
        assertEq(OfferBoard.slashable.selector, bytes4(0x4bff9a80));
        assertEq(OfferBoard.expired.selector, bytes4(0xba065e1f));
        assertEq(OfferBoard.custodian.selector, bytes4(0x375b74c3));
    }

    /// The same bytes `handin.rs` decodes in its tests (`cast abi-encode`, pinned there as hex).
    function test_submissionOfEncodesAsTheCanisterDecodes() public {
        vm.warp(1_800_000_000);
        vm.roll(26_000_000);
        address who = 0x1111111111111111111111111111111111111111;
        vm.deal(who, 1 ether);
        vm.prank(who);
        uint256 id = board.submit{value: BOND}(bytes32(uint256(0x2222222222222222222222222222222222222222222222222222222222222222)), 903_000, ID);
        (bool ok, bytes memory ret) = address(board).staticcall(abi.encodeWithSelector(OfferBoard.submissionOf.selector, id));
        assertTrue(ok);
        assertEq(
            ret,
            hex"00000000000000000000000000000000000000000000000000000000000000200000000000000000000000001111111111111111111111111111111111111111000000000000000000000000000000000000000000000000000000006b49d20000000000000000000000000000000000000000000000000000000000018cba800000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000002386f26fc1000000000000000000000000000000000000000000000000000000000000000dc758222222222222222222222222222222222222222222222222222222222222222200000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000042366662393736616234396463656330313766316532303165383433393539383332303461653161376332616266376365643061383564363932653434323739396930000000000000000000000000000000000000000000000000000000000000"
        );
        (ok, ret) = address(board).staticcall(abi.encodeWithSelector(OfferBoard.params.selector));
        assertTrue(ok);
        assertEq(
            ret,
            hex"000000000000000000000000000000000000000000000000002386f26fc10000000000000000000000000000000000000000000000000000002386f26fc100000000000000000000000000000000000000000000000000000000000000000e10000000000000000000000000000000000000000000000000000000000001518000000000000000000000000000000000000000000000000002c68af0bb1400000000000000000000000000000000000000000000000000000000000000000040"
        );
    }

    function test_openIdsIsAPlainArrayAndTracksCloses() public {
        uint256 a = _submit(bot, bytes32(uint256(1)), 900_000);
        uint256 b = _submit(bot, bytes32(uint256(2)), 950_000);
        uint256 c = _submit(other, bytes32(uint256(3)), 800_000);
        uint256[] memory ids = board.openIds(0, 64);
        assertEq(ids.length, 3);
        assertEq(ids[0], a); assertEq(ids[1], b); assertEq(ids[2], c);
        // the custodian frees the middle one: swap-remove keeps the list dense and the count right
        vm.prank(custodian);
        board.refund(b);
        ids = board.openIds(0, 64);
        assertEq(ids.length, 2);
        assertEq(ids[0], a); assertEq(ids[1], c);
        assertEq(board.openCount(), 2);
        ids = board.openIds(1, 1);
        assertEq(ids.length, 1); assertEq(ids[0], c);
        ids = board.openIds(5, 64);
        assertEq(ids.length, 0);
        assertEq(board.openIdOf(bytes32(uint256(2))), 0, "a closed hash is free again");
        assertEq(board.openIdOf(bytes32(uint256(1))), a + 1);
    }

    // ---- submitting -------------------------------------------------------------------------------------------------------------------

    function test_aSubmissionTakesExactlyTheBondAndIsRecordedWithBothClocks() public {
        uint256 id = _submit(bot, bytes32(uint256(1)), 900_000);
        OfferBoard.Submission memory s = board.submissionOf(id);
        assertEq(s.submitter, bot);
        assertEq(s.at, 1_800_000_000);
        assertEq(s.atBlock, 26_000_000);
        assertEq(uint8(s.status), uint8(OfferBoard.Status.Open));
        assertEq(s.bond, BOND);
        assertEq(s.askSats, 900_000);
        assertEq(s.offerHash, bytes32(uint256(1)));
        assertEq(s.inscriptionId, ID);
        assertEq(board.bonded(), BOND);
        assertEq(board.openBy(bot), 1);
        _books();
        vm.prank(bot);
        vm.expectRevert(abi.encodeWithSelector(OfferBoard.WrongBond.selector, BOND - 1, BOND));
        board.submit{value: BOND - 1}(bytes32(uint256(9)), 1, ID);
        vm.prank(bot);
        vm.expectRevert(abi.encodeWithSelector(OfferBoard.WrongBond.selector, BOND + 1, BOND));
        board.submit{value: BOND + 1}(bytes32(uint256(9)), 1, ID);
    }

    function test_oneOpenSubmissionPerHashAndTheRateLimits() public {
        uint256 id = _submit(bot, bytes32(uint256(1)), 900_000);
        vm.prank(other);
        vm.expectRevert(abi.encodeWithSelector(OfferBoard.Duplicate.selector, id));
        board.submit{value: BOND}(bytes32(uint256(1)), 900_000, ID);
        // per address
        for (uint256 i = 2; i <= PER_ADDR; i++) _submit(bot, bytes32(i), 900_000);
        vm.prank(bot);
        vm.expectRevert(abi.encodeWithSelector(OfferBoard.TooManyOpen.selector, bot));
        board.submit{value: BOND}(bytes32(uint256(99)), 900_000, ID);
        // a closed one frees the address's count and the hash
        Clock.skip(EXPIRY);
        vm.prank(bot);
        board.refund(id);
        _submit(bot, bytes32(uint256(1)), 900_000);
        // bad inputs
        vm.prank(other);
        vm.expectRevert(OfferBoard.BadInput.selector);
        board.submit{value: BOND}(bytes32(0), 900_000, ID);
        vm.prank(other);
        vm.expectRevert(OfferBoard.BadInput.selector);
        board.submit{value: BOND}(bytes32(uint256(77)), 0, ID);
        vm.prank(other);
        vm.expectRevert(OfferBoard.BadInput.selector);
        board.submit{value: BOND}(bytes32(uint256(77)), 1, "");
    }

    function test_theBoardIsBoundedInAll() public {
        OfferBoard small = new OfferBoard(IWrappedCollection(address(wrapper)), BOND, REWARD, SLASH_DELAY, EXPIRY, DAY_CAP, 2, 2, PBS);
        vm.startPrank(bot);
        small.submit{value: BOND}(bytes32(uint256(1)), 1, ID);
        small.submit{value: BOND}(bytes32(uint256(2)), 1, ID);
        vm.stopPrank();
        vm.prank(other);
        vm.expectRevert(OfferBoard.BoardFull.selector);
        small.submit{value: BOND}(bytes32(uint256(3)), 1, ID);
    }

    // ---- claiming -------------------------------------------------------------------------------------------------------------------

    function test_aClaimCreditsBondPlusRewardOnceAndOnlyTheCustodianMay() public {
        _fund(1 ether);
        uint256 id = _submit(bot, bytes32(uint256(1)), 900_000);
        vm.prank(bot);
        vm.expectRevert(OfferBoard.NotCustodian.selector);
        board.claim(id);
        vm.prank(custodian);
        uint256 paid = board.claim(id);
        assertEq(paid, REWARD);
        assertEq(board.owed(bot), BOND + REWARD);
        assertEq(board.rewardPool(), 1 ether - REWARD);
        assertEq(board.bonded(), 0);
        assertEq(board.openCount(), 0);
        assertEq(uint8(board.submissionOf(id).status), uint8(OfferBoard.Status.Claimed));
        _books();
        // never twice, and never after
        vm.prank(custodian);
        vm.expectRevert(OfferBoard.WrongStatus.selector);
        board.claim(id);
        vm.prank(custodian);
        vm.expectRevert(OfferBoard.WrongStatus.selector);
        board.slash(id);
        vm.prank(bot);
        vm.expectRevert(OfferBoard.WrongStatus.selector);
        board.refund(id);
        // and the bot pulls it
        uint256 before = bot.balance;
        vm.prank(bot);
        assertEq(board.withdraw(), BOND + REWARD);
        assertEq(bot.balance, before + BOND + REWARD);
        assertEq(board.owedTotal(), 0);
        assertEq(address(board).balance, 1 ether - REWARD);
        _books();
    }

    function test_aDryPoolStillReturnsTheBondAndPaysWhatIsThere() public {
        uint256 id = _submit(bot, bytes32(uint256(1)), 900_000);
        vm.prank(custodian);
        assertEq(board.claim(id), 0, "nothing in the pool: no reward, no revert");
        assertEq(board.owed(bot), BOND);
        _fund(REWARD / 4);
        uint256 id2 = _submit(bot, bytes32(uint256(2)), 900_000);
        vm.prank(custodian);
        assertEq(board.claim(id2), REWARD / 4, "a quarter is what there is");
        assertEq(board.rewardPool(), 0);
        _books();
    }

    function test_theDayCapBoundsRewardsAndRollsOnBothClocks() public {
        _fund(10 ether);
        // 0.2 ETH a day is twenty rewards; the twenty-first pays nothing until a day passes on both clocks
        uint256 n = DAY_CAP / REWARD;
        for (uint256 i = 1; i <= n; i++) {
            // spread over addresses: four open per address
            address who = address(uint160(0x1000 + i));
            vm.deal(who, BOND);
            vm.prank(who);
            uint256 id = board.submit{value: BOND}(bytes32(i), 900_000, ID);
            vm.prank(custodian);
            assertEq(board.claim(id), REWARD);
        }
        assertEq(board.paidToday(), DAY_CAP);
        uint256 late = _submit(bot, bytes32(uint256(1000)), 900_000);
        vm.prank(custodian);
        assertEq(board.claim(late), 0, "the day's cap");
        // the sequencer's clock alone does not roll the day
        Clock.skipTimeOnly(1 days + 1);
        uint256 late2 = _submit(bot, bytes32(uint256(1001)), 900_000);
        vm.prank(custodian);
        assertEq(board.claim(late2), 0, "time alone: still the same day");
        Clock.skip(1 days);
        uint256 fresh = _submit(bot, bytes32(uint256(1002)), 900_000);
        vm.prank(custodian);
        assertEq(board.claim(fresh), REWARD, "a new day on both clocks");
        assertEq(board.paidToday(), REWARD);
        _books();
    }

    function test_aSubmitterThatRefusesEthCannotBlockTheClaim() public {
        _fund(1 ether);
        RefusingSubmitter r = new RefusingSubmitter(board);
        vm.deal(address(r), BOND);
        uint256 id = r.submit{value: BOND}(bytes32(uint256(1)), 900_000);
        vm.prank(custodian);
        assertEq(board.claim(id), REWARD, "the claim lands; only its own withdraw fails");
        vm.expectRevert(OfferBoard.TransferFailed.selector);
        r.pull();
        _books();
    }

    // ---- slashing -------------------------------------------------------------------------------------------------------------------

    function test_aSlashWaitsForTheDelayOnBothClocksThenMovesTheBondToThePool() public {
        uint256 id = _submit(bot, bytes32(uint256(1)), 900_000);
        assertFalse(board.slashable(id));
        vm.prank(custodian);
        vm.expectRevert(OfferBoard.TooEarly.selector);
        board.slash(id);
        // the sequencer stamping an hour ahead does not make it slashable
        Clock.skipTimeOnly(SLASH_DELAY);
        assertFalse(board.slashable(id));
        vm.prank(custodian);
        vm.expectRevert(OfferBoard.TooEarly.selector);
        board.slash(id);
        Clock.skip(SLASH_DELAY);
        assertTrue(board.slashable(id));
        vm.prank(bot);
        vm.expectRevert(OfferBoard.NotCustodian.selector);
        board.slash(id);
        vm.prank(custodian);
        board.slash(id);
        assertEq(board.rewardPool(), BOND);
        assertEq(board.totalSlashed(), BOND);
        assertEq(board.owed(bot), 0);
        assertEq(board.bonded(), 0);
        assertEq(uint8(board.submissionOf(id).status), uint8(OfferBoard.Status.Slashed));
        assertFalse(board.slashable(id), "closed");
        _books();
        // the slashed bond funds the next reward
        uint256 id2 = _submit(other, bytes32(uint256(2)), 900_000);
        vm.prank(custodian);
        assertEq(board.claim(id2), REWARD);
        assertEq(board.rewardPool(), 0);
    }

    // ---- refunds -------------------------------------------------------------------------------------------------------------------

    function test_aValidUnboughtHandInIsRefundedByItsSubmitterAfterExpiryOnBothClocks() public {
        uint256 id = _submit(bot, bytes32(uint256(1)), 900_000);
        vm.prank(bot);
        vm.expectRevert(OfferBoard.TooEarly.selector);
        board.refund(id);
        Clock.skipTimeOnly(EXPIRY);
        assertFalse(board.expired(id));
        vm.prank(bot);
        vm.expectRevert(OfferBoard.TooEarly.selector);
        board.refund(id);
        Clock.skip(EXPIRY);
        assertTrue(board.expired(id));
        vm.prank(other);
        vm.expectRevert(OfferBoard.NotSubmitter.selector);
        board.refund(id);
        vm.prank(bot);
        board.refund(id);
        assertEq(board.owed(bot), BOND, "the bond, no reward");
        assertEq(uint8(board.submissionOf(id).status), uint8(OfferBoard.Status.Refunded));
        _books();
        // once refunded the custodian can neither claim nor slash it
        vm.prank(custodian);
        vm.expectRevert(OfferBoard.WrongStatus.selector);
        board.claim(id);
        vm.prank(custodian);
        vm.expectRevert(OfferBoard.WrongStatus.selector);
        board.slash(id);
    }

    function test_theCustodianMayRefundAtAnyTimeAndTheSlashWindowClosesAtExpiry() public {
        uint256 id = _submit(bot, bytes32(uint256(1)), 900_000);
        vm.prank(custodian);
        board.refund(id);
        assertEq(board.owed(bot), BOND);
        // a submitter who is not slashed before expiry takes the bond back; the slash then fails
        uint256 id2 = _submit(bot, bytes32(uint256(2)), 900_000);
        Clock.skip(EXPIRY);
        vm.prank(bot);
        board.refund(id2);
        vm.prank(custodian);
        vm.expectRevert(OfferBoard.WrongStatus.selector);
        board.slash(id2);
        _books();
    }

    function test_theCustodianFollowsTheWrapper() public {
        uint256 id = _submit(bot, bytes32(uint256(1)), 900_000);
        address next = makeAddr("the next custodian");
        wrapper.set(next);
        assertEq(board.custodian(), next);
        vm.prank(custodian);
        vm.expectRevert(OfferBoard.NotCustodian.selector);
        board.claim(id);
        vm.prank(next);
        board.claim(id);
    }

    function test_constructorRefusesWhatCannotWork() public {
        IWrappedCollection w = IWrappedCollection(address(wrapper));
        vm.expectRevert(OfferBoard.BadInput.selector);
        new OfferBoard(IWrappedCollection(address(0)), BOND, REWARD, SLASH_DELAY, EXPIRY, DAY_CAP, MAX_OPEN, PER_ADDR, PBS);
        vm.expectRevert(OfferBoard.BadInput.selector);
        new OfferBoard(w, 0, REWARD, SLASH_DELAY, EXPIRY, DAY_CAP, MAX_OPEN, PER_ADDR, PBS);
        vm.expectRevert(OfferBoard.BadInput.selector);
        new OfferBoard(w, BOND, REWARD, SLASH_DELAY, SLASH_DELAY + 1 hours, DAY_CAP, MAX_OPEN, PER_ADDR, PBS);
        vm.expectRevert(OfferBoard.BadInput.selector);
        new OfferBoard(w, BOND, REWARD, 0, EXPIRY, DAY_CAP, MAX_OPEN, PER_ADDR, PBS);
        vm.expectRevert(OfferBoard.BadInput.selector);
        new OfferBoard(w, BOND, REWARD, SLASH_DELAY, EXPIRY, DAY_CAP, 4, 5, PBS);
        vm.expectRevert();
        new OfferBoard(w, BOND, REWARD, SLASH_DELAY, EXPIRY, DAY_CAP, MAX_OPEN, PER_ADDR, 0);
    }

    function test_fundNeedsValue() public {
        vm.prank(funder);
        vm.expectRevert(OfferBoard.BadInput.selector);
        board.fund();
        assertEq(board.totalFunded(), 0);
    }
}
