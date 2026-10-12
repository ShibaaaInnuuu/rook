// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Clock} from "../utils/Clock.sol";
import {OfferBoard} from "../../src/handin/OfferBoard.sol";
import {IWrappedCollection} from "../../src/interfaces/IWrappedCollection.sol";
import {MockCustodied} from "./OfferBoard.t.sol";

/// Bots submit, the custodian claims and slashes and refunds, submitters refund and withdraw, the pool is funded, and time
/// passes on both clocks — in any order.
contract BoardHandler is Test {
    OfferBoard public b;
    address custodian;
    address[] public bots;
    uint256 public nextHash = 1;
    uint256 public panics;
    /// ghosts: what went in, what was credited, how many times each id was paid
    uint256 public ghostFunded;
    uint256 public ghostBondsIn;
    uint256 public ghostCredited;
    uint256 public ghostWithdrawn;
    uint256 public ghostRewards;
    mapping(uint256 id => uint256) public paidTimes;

    constructor(OfferBoard b_, address custodian_) {
        b = b_;
        custodian = custodian_;
        for (uint256 i = 0; i < 6; i++) {
            bots.push(address(uint160(0xb0b0 + i)));
        }
    }

    function _check(bytes memory err) internal {
        if (err.length >= 4 && bytes4(err) == bytes4(0x4e487b71)) panics++;
    }

    function submit(uint8 who, uint64 ask) public {
        address bot = bots[who % bots.length];
        uint256 a = bound(uint256(ask), 1, 5_000_000);
        vm.deal(bot, bot.balance + b.bond());
        vm.prank(bot);
        try b.submit{value: b.bond()}(bytes32(nextHash++), a, "aaaa1111i0") {
            ghostBondsIn += b.bond();
        } catch (bytes memory err) { _check(err); }
    }

    function claim(uint256 which) public {
        uint256 n = b.submissionCount();
        if (n == 0) return;
        uint256 id = which % n;
        uint256 pool = b.rewardPool();
        vm.prank(custodian);
        try b.claim(id) returns (uint256 paid) {
            paidTimes[id] += 1;
            ghostRewards += paid;
            ghostCredited += b.bond() + paid;
            assertLe(paid, b.reward());
            assertLe(paid, pool);
        } catch (bytes memory err) { _check(err); }
    }

    function slash(uint256 which) public {
        uint256 n = b.submissionCount();
        if (n == 0) return;
        uint256 id = which % n;
        vm.prank(custodian);
        try b.slash(id) {} catch (bytes memory err) { _check(err); }
    }

    function refund(uint256 which, bool byCustodian) public {
        uint256 n = b.submissionCount();
        if (n == 0) return;
        uint256 id = which % n;
        OfferBoard.Submission memory s = b.submissionOf(id);
        vm.prank(byCustodian ? custodian : s.submitter);
        try b.refund(id) { ghostCredited += s.bond; } catch (bytes memory err) { _check(err); }
    }

    function withdraw(uint8 who) public {
        address bot = bots[who % bots.length];
        vm.prank(bot);
        try b.withdraw() returns (uint256 amount) { ghostWithdrawn += amount; } catch (bytes memory err) { _check(err); }
    }

    function fund(uint96 amount) public {
        uint256 a = bound(uint256(amount), 1, 1 ether);
        vm.deal(address(this), a);
        try b.fund{value: a}() { ghostFunded += a; } catch (bytes memory err) { _check(err); }
    }

    function passTime(uint32 secs) public {
        Clock.skip(bound(secs, 1, 2 days));
    }

    function botCount() external view returns (uint256) { return bots.length; }
}

contract OfferBoardInvariantTest is Test {
    OfferBoard board;
    BoardHandler h;
    address custodian = makeAddr("custodian");

    function setUp() public {
        vm.warp(1_800_000_000);
        vm.roll(26_000_000);
        MockCustodied w = new MockCustodied(custodian);
        board = new OfferBoard(IWrappedCollection(address(w)), 0.01 ether, 0.01 ether, 1 hours, 24 hours, 0.05 ether, 16, 4, 12);
        h = new BoardHandler(board, custodian);
        targetContract(address(h));
    }

    /// `balance == bonded + rewardPool + owedTotal`, always.
    function invariant_theBooksBalance() public view {
        assertEq(address(board).balance, board.bonded() + board.rewardPool() + board.owedTotal());
        assertEq(h.panics(), 0, "no arithmetic panic anywhere");
    }

    /// Rewards never exceed the budget: what was funded plus what was slashed.
    function invariant_rewardsNeverExceedTheBudget() public view {
        assertLe(board.totalRewardsPaid(), board.totalFunded() + board.totalSlashed());
        assertEq(board.totalRewardsPaid(), h.ghostRewards());
        assertEq(board.rewardPool(), board.totalFunded() + board.totalSlashed() - board.totalRewardsPaid());
        assertLe(board.paidToday(), board.rewardDayCap());
    }

    /// A submitter is never paid twice for one submission: every id is credited at most once, and only open ones are counted.
    function invariant_eachIdIsPaidAtMostOnce() public view {
        uint256 n = board.submissionCount();
        for (uint256 id = 0; id < n; id++) {
            assertLe(h.paidTimes(id), 1);
            OfferBoard.Submission memory s = board.submissionOf(id);
            if (h.paidTimes(id) == 1) assertEq(uint8(s.status), uint8(OfferBoard.Status.Claimed));
        }
    }

    /// Bonds are conserved: every bond that came in is still bonded, or credited to its submitter, or in the pool (slashed);
    /// `bonded` is exactly the open submissions' bonds; the open count is the sum per submitter and the list is dense.
    function invariant_bondsAreConserved() public view {
        uint256 n = board.submissionCount();
        uint256 openBonds;
        uint256 openN;
        uint256 slashed;
        for (uint256 id = 0; id < n; id++) {
            OfferBoard.Submission memory s = board.submissionOf(id);
            if (s.status == OfferBoard.Status.Open) { openBonds += s.bond; openN++; }
            if (s.status == OfferBoard.Status.Slashed) slashed += s.bond;
        }
        assertEq(board.bonded(), openBonds);
        assertEq(board.openCount(), openN);
        assertEq(board.openIds(0, 1_000).length, openN);
        assertEq(slashed, board.totalSlashed());
        assertEq(h.ghostBondsIn(), openBonds + slashed + (h.ghostCredited() - h.ghostRewards()));
        uint256 byBots;
        for (uint256 i = 0; i < h.botCount(); i++) byBots += board.openBy(h.bots(i));
        assertEq(byBots, openN);
        assertEq(board.owedTotal(), h.ghostCredited() - h.ghostWithdrawn());
    }
}
