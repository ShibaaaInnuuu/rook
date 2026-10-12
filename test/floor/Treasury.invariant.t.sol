// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {TreasuryBase} from "./Treasury.t.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";
import {ProtocolTreasury} from "../../src/floor/ProtocolTreasury.sol";

/// Drives the treasury and its one collection: income from anywhere, allocations, distributions, the bootstrap loan, fee flow
/// through the module (which repays first), the timelock's share changes and forgiveness — in any order.
contract TreasuryHandler is Test {
    ProtocolTreasury immutable t;
    FloorModule immutable module;
    address immutable owner;
    address immutable keeper;
    uint256 constant F = 0.633 ether;

    // ghosts
    uint256 public repaidSeen;          // the loan's `repaid`, which may only grow
    bool public repaidFell;
    bool public outstandingFellOtherwise; // outstanding fell without a repayment or a forgiveness
    uint256 public lastOutstanding;
    bool public forgivenSeen;
    uint256 public calls;

    constructor(ProtocolTreasury t_, FloorModule m, address o, address k) {
        t = t_; module = m; owner = o; keeper = k;
    }

    receive() external payable {}

    function _fresh() private {
        if (!module.fresh()) {
            vm.prank(keeper);
            module.postFloor(F, 10_000, 50);
        }
    }

    function _check() private {
        ProtocolTreasury.Loan memory l = t.loan(address(module));
        if (l.repaid < repaidSeen) repaidFell = true;
        repaidSeen = l.repaid;
        uint256 due = t.outstanding(address(module));
        if (due < lastOutstanding && l.repaid == repaidSeen && !l.forgiven && due != l.principal - l.repaid) outstandingFellOtherwise = true;
        lastOutstanding = due;
        if (l.forgiven) forgivenSeen = true;
    }

    function income(uint256 amount) external {
        calls++;
        amount = bound(amount, 1, 20 ether);
        vm.deal(address(this), address(this).balance + amount);
        (bool ok,) = address(t).call{value: amount}("");
        require(ok);
        _check();
    }

    function allocate() external {
        calls++;
        if (t.unallocated() == 0) return;
        t.allocate();
        _check();
    }

    function distribute(uint256 secs) external {
        calls++;
        vm.warp(vm.getBlockTimestamp() + bound(secs, 0, 2 hours));
        vm.roll(vm.getBlockNumber() + bound(secs, 0, 2 hours) / 12 + 1);
        if (module.volume7d() == 0) {
            _fresh();
            vm.deal(address(this), address(this).balance + 0.1 ether);
            module.mint{value: 0.1 ether}(address(this), 0);
        }
        if (t.reservesPot() + t.floatsPot() == 0) return;
        t.distribute();
        _check();
    }

    function bootstrap() external {
        calls++;
        if (t.loan(address(module)).principal != 0 || t.bootstrapPot() == 0) return;
        t.bootstrap(module);
        lastOutstanding = t.outstanding(address(module));
        _check();
    }

    function feeFlow(uint256 amount) external {
        calls++;
        amount = bound(amount, 1, 1 ether);
        vm.deal(address(this), address(this).balance + amount);
        uint256 dueBefore = t.outstanding(address(module));
        module.fundReserve{value: amount}();
        uint256 dueAfter = t.outstanding(address(module));
        if (dueBefore - dueAfter != (amount < dueBefore ? amount : dueBefore)) outstandingFellOtherwise = true;
        lastOutstanding = dueAfter;
        _check();
    }

    function forgive() external {
        calls++;
        if (t.outstanding(address(module)) == 0) return;
        vm.prank(owner);
        t.forgive(address(module));
        lastOutstanding = 0;
        _check();
    }

    function setShares(uint16 a, uint16 b, uint16 c) external {
        calls++;
        a = uint16(bound(a, 0, 10_000)); b = uint16(bound(b, 0, 10_000 - a)); c = uint16(bound(c, 0, 10_000 - a - b));
        vm.prank(owner);
        t.setShares(a, b, c, 10_000 - a - b - c);
        _check();
    }
}

/// forge-config: default.invariant.fail-on-revert = true
contract TreasuryInvariantTest is TreasuryBase {
    TreasuryHandler handler;

    function setUp() public override {
        super.setUp();
        handler = new TreasuryHandler(t, module, owner, keeper);
        targetContract(address(handler));
    }

    /// The shares sum to 100% at every step.
    function invariant_theSharesSumToOne() public view {
        assertEq(uint256(t.reservesBps()) + t.floatsBps() + t.bootstrapBps() + t.opsBps(), 10_000);
    }

    /// The pots never exceed the balance: nothing is allocated that is not held.
    function invariant_thePotsNeverExceedTheBalance() public view {
        assertLe(t.reservesPot() + t.floatsPot() + t.bootstrapPot(), address(t).balance);
        assertEq(address(t).balance, t.reservesPot() + t.floatsPot() + t.bootstrapPot() + t.unallocated());
    }

    /// Every wei that left went to the collection or to operations; the treasury's own books say so.
    function invariant_everyWeiOutIsAccounted() public view {
        uint256 outflow = t.totalToOps() + t.totalToReserves() + t.totalToFloats() + t.totalLent();
        assertEq(address(t).balance + outflow, t.totalReceived(), "in == held + out");
    }

    /// A loan is repaid at most once and falls only by repayment or forgiveness.
    function invariant_loansAreMonotone() public view {
        ProtocolTreasury.Loan memory l = t.loan(address(module));
        assertLe(l.repaid, l.principal);
        assertFalse(handler.repaidFell());
        assertFalse(handler.outstandingFellOtherwise());
        assertEq(t.outstanding(address(module)), l.forgiven ? 0 : l.principal - l.repaid);
    }

    /// The module's books hold with the treasury's funding in them.
    function invariant_theModulesBooksHold() public view {
        _books();
        assertEq(module.netUnits() + module.reserveUnits(), wrapper.ethBacked());
    }
}
