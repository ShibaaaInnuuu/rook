// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {ListingRegistry} from "../../../src/ListingRegistry.sol";
import {ProtocolTreasury} from "../../../src/floor/ProtocolTreasury.sol";
import {FloorModule} from "../../../src/floor/FloorModule.sol";

/// A module that accepts whatever the treasury sends and reports a volume (the PoC's stand-in for the real module's entries).
contract StubModule {
    address public treasury;
    uint256 public vol;
    bool public revertReserve;
    uint256 public floats;
    uint256 public reserves;
    constructor(address t, uint256 v) { treasury = t; vol = v; }
    function volume7d() external view returns (uint256) { return vol; }
    function fundReserve() external payable { require(!revertReserve, "reserve refused"); reserves += msg.value; }
    function fundFloat() external payable returns (uint256) { floats += msg.value; return msg.value; }
    function setRevertReserve(bool b) external { revertReserve = b; }
    function sweep(address to) external { (bool ok,) = to.call{value: address(this).balance}(""); require(ok); }
}

/// Audit C regressions for the treasury (docs/audit/2026-09-26/C-treasury-roles-deploy.md C-06, C-07, C-12; PoCs in
/// docs/audit/2026-09-26/poc/C). Design: docs/audit/2026-09-26/fixes-B-C-keeper-draws.md.
contract C_ProtocolTreasuryRegression is Test {
    address safe = makeAddr("safe (2-of-2)");
    address eoaTreasury = makeAddr("registry treasury EOA");
    address ops = makeAddr("ops");
    address attacker = makeAddr("holder of both devices");
    ListingRegistry registry;
    ProtocolTreasury t;
    StubModule module;

    function setUp() public {
        registry = new ListingRegistry(safe, eoaTreasury);
        t = new ProtocolTreasury(registry);
        module = new StubModule(address(t), 1 ether);
    }

    function _income(uint256 amount) internal {
        vm.deal(address(this), amount);
        (bool ok,) = address(t).call{value: amount}("");
        require(ok);
    }

    /// C-07: Phase 1's split is the treasury's from construction; nothing at go-live can forget it. Income before any owner step
    /// funds no float and no loan, and nobody can open a bootstrap loan (the PoC opened a 2 ETH one at the defaults).
    function test_C07_phase1IsTheDefault_noFloatNoLoan() public {
        assertEq(t.reservesBps(), 9_000);
        assertEq(t.floatsBps(), 0);
        assertEq(t.bootstrapBps(), 0);
        assertEq(t.opsBps(), 1_000);
        assertEq(t.bootstrapLoan(), 0);
        vm.prank(safe);
        t.addCollection(FloorModule(payable(address(module))));
        _income(20 ether);                                    // a sweep-only launch trades before step 7
        t.allocate();
        assertEq(t.floatsPot(), 0);
        assertEq(t.bootstrapPot(), 0);
        assertEq(t.reservesPot(), 20 ether, "90% plus the unset operations share");
        vm.prank(makeAddr("anyone"));
        vm.expectRevert(ProtocolTreasury.NothingToDo.selector);
        t.bootstrap(FloorModule(payable(address(module))));
        t.distribute();
        assertEq(module.floats(), 0, "no float");
        assertEq(module.reserves(), 20 ether);
    }

    /// C-07 (b): a route switched off is never paid from income that arrived before the switch: its pot moves to the reserves.
    function test_C07b_switchingARouteOffMovesItsPotToTheReserves() public {
        vm.startPrank(safe);
        t.addCollection(FloorModule(payable(address(module))));
        t.setShares(4_000, 3_000, 2_000, 1_000);              // stage 1
        vm.stopPrank();
        _income(1 ether);
        t.allocate();
        assertEq(t.floatsPot(), 0.3 ether);
        assertEq(t.bootstrapPot(), 0.2 ether);
        vm.prank(safe);
        t.setShares(9_000, 0, 0, 1_000);                      // back to Phase 1
        assertEq(t.floatsPot(), 0);
        assertEq(t.bootstrapPot(), 0);
        assertEq(t.reservesPot(), 1 ether, "the reserves pot took both (ops unset: its share joined too)");
        t.distribute();
        assertEq(module.floats(), 0, "the PoC's float after floats were switched off: gone");
    }

    /// C-12: a registered module whose `fundReserve` reverts keeps its share in the pot; the others are paid.
    function test_C12_oneRevertingModuleNoLongerBlocksDistribute() public {
        StubModule other = new StubModule(address(t), 1 ether);
        vm.startPrank(safe);
        t.addCollection(FloorModule(payable(address(module))));
        t.addCollection(FloorModule(payable(address(other))));
        vm.stopPrank();
        _income(1 ether);
        t.allocate();
        other.setRevertReserve(true);
        t.distribute();
        assertEq(module.reserves(), 0.5 ether, "the healthy module was paid");
        assertEq(t.reservesPot(), 0.5 ether, "the refusing module's share stays in the pot");
    }

    /// C-06 is NOT changed (a legitimate Safe power, documented for the Trust page): holding both devices, the Safe can still take
    /// every pot at once. This pins the statement the Trust page must make.
    function test_C06_documented_theSafeCanStillTakeEveryPotAtOnce() public {
        vm.startPrank(safe);
        t.addCollection(FloorModule(payable(address(module))));
        t.setOps(ops);
        t.setShares(4_000, 3_000, 2_000, 1_000);
        t.setBootstrapLoan(2 ether);
        vm.stopPrank();
        _income(10 ether);
        t.allocate();
        _income(5 ether);
        vm.startPrank(safe);
        StubModule fake = new StubModule(address(t), type(uint128).max);
        t.addCollection(FloorModule(payable(address(fake))));
        t.setShares(0, 0, 0, 10_000);
        t.setOps(attacker);
        t.setBootstrapLoan(type(uint256).max);
        vm.stopPrank();
        t.allocate();                                         // (the floats and bootstrap pots moved to reserves at setShares)
        t.distribute();
        fake.sweep(attacker);
        assertApproxEqAbs(attacker.balance, 14 ether, 1e9, "the whole balance: the Trust page says so");
    }
}
