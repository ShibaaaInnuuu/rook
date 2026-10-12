// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SweepBase} from "../utils/SweepBase.sol";
import {Merkle} from "../utils/Merkle.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";
import {IKeeperSource} from "../../src/interfaces/IKeeperSource.sol";
import {IFloorWrapped} from "../../src/interfaces/IFloorWrapped.sol";
import {Wrapper} from "../../src/Wrapper.sol";

/// `SweepBase` (the frog wrapper, 64 frogs, the sweep vaults) plus the collection's floor module, wired before anything exists,
/// with the frog defaults of docs/v5/floor-module.md §6: 15 bps base, 1× volatility, +1% stale, +3% stress, 10% max, ±5%/h, 3%
/// conversion, 5% buffer, 5 frogs an hour each way, 10 frogs of public ETH backing (40 hard). The keeper posts a 0.633 ETH floor.
/// The reserve share (§13.1) is ZERO here, so these suites test the convertible machinery alone; `FloorReserve.t.sol` and the
/// invariant suite run the 25% default, and `FloorRouter.t.sol` runs it against the real pools. No treasury (`address(0)`) unless
/// a suite sets `moduleTreasury` before `setUp`.
abstract contract FloorBase is SweepBase {
    FloorModule module;
    address platform = makeAddr("platform");
    /// The protocol treasury the module is built with (docs/v5/floor-module.md §13.3); zero here, set by the treasury suites.
    address moduleTreasury;
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    address renter = makeAddr("renter");

    uint256 constant F = 0.633 ether;
    bytes32[] rentLeaves;         // the keeper's eligibility list: frogs 32..63 (the tests' "floor" frogs)
    bytes32 rentRoot;

    function _params() internal pure virtual returns (FloorModule.Params memory p) {
        p.baseSpreadBps = 15;
        p.volWeightBps = 10_000;
        p.staleSpreadBps = 100;
        p.stressSpreadBps = 300;
        p.maxSpreadBps = 1_000;
        p.maxMoveBps = 500;
        p.convBps = 300;
        p.bufferBps = 500;
        p.mintPerHour = 5e18;
        p.burnPerHour = 5e18;
        p.maxPublicEthBacked = 10e18;
        p.maxEthBacked = 40e18;
        p.revenueTarget = 1 ether;
        p.rentTerm = 30 days;
        p.premiumBps = 1_000;
        p.rentShareBps = 5_000;
        p.maxRentals = 20;
        p.lowConfidenceSpreadBps = 100;
        p.fullConfidenceBps = 8_000;
        p.minConfidenceBps = 3_000;
        p.maxItemsPerDay = 40;
        p.reserveBps = 0;
        p.fillBandBps = 300;
    }

    function setUp() public virtual override {
        super.setUp();
        module = new FloorModule(IFloorWrapped(address(wrapper)), IKeeperSource(address(registry)), payee, platform, moduleTreasury, _params(), PBS);
        wrapper.wireFloorModule(address(module));
        for (uint256 i = 32; i < N_FROGS; ++i) rentLeaves.push(keccak256(abi.encode(frogLeaves[i])));
        rentRoot = Merkle.root(rentLeaves);
        vm.prank(keeper);
        module.postRentalRoot(rentRoot);
        _post(F);
    }

    /// Both clocks forward by `secs` (the parent chain at 12 s a block, one block over). Unlike `Clock.skip` this reads the
    /// block number through the cheatcode: under via-IR a `block.number` read can be cached across a loop in one test function.
    function _skip(uint256 secs) internal {
        vm.warp(vm.getBlockTimestamp() + secs);
        vm.roll(vm.getBlockNumber() + secs / PBS + 1);
    }

    /// Only the sequencer's clock: what a sequencer stamping blocks ahead can do.
    function _skipTimeOnly(uint256 secs) internal {
        vm.warp(vm.getBlockTimestamp() + secs);
    }

    /// Replace the wrapper and the module with fresh ones built with `p` (a wrapper's module is wired once, before anything exists).
    function _redeploy(FloorModule.Params memory p) internal {
        wrapper = new Wrapper(registry, distributor, beacon, FROGS, distribution, custodian, CUSTODY_BTC, "w2", "w2", PBS);
        wrapper.initMintLimit(1_000);
        module = new FloorModule(IFloorWrapped(address(wrapper)), IKeeperSource(address(registry)), payee, platform, moduleTreasury, p, PBS);
        wrapper.wireFloorModule(address(module));
        vm.prank(keeper);
        module.postRentalRoot(rentRoot);
        _post(F);
    }

    /// The keeper's confidence and depth in most tests: a full book (no confidence spread, depth not binding).
    uint16 conf = 10_000;
    uint32 depth = 50;

    function _post(uint256 price) internal {
        vm.prank(keeper);
        module.postFloor(price, conf, depth);
    }

    /// Post `price` and keep posting it, an hour at a time, until the accepted floor reaches it (the move limit).
    function _walkTo(uint256 price) internal {
        for (uint256 i = 0; i < 200 && module.accepted() != price; ++i) {
            _skip(1 hours);
            _post(price);
        }
        assertEq(module.accepted(), price, "walked to the price");
    }

    /// Let the volatility decay and re-post the same floor: a calm market.
    function _calm() internal {
        _skip(1 hours);
        _post(module.accepted());
    }

    function _mint(address who, uint256 eth) internal returns (uint256 units) {
        vm.deal(who, who.balance + eth);
        vm.prank(who);
        units = module.mint{value: eth}(who, 0);
    }

    function _burn(address who, uint256 units) internal returns (uint256 eth) {
        vm.startPrank(who);
        wrapper.approve(address(module), units);
        eth = module.burn(units, 0, who);
        vm.stopPrank();
    }

    /// The canister buys one item and wraps it to the module, then records `cost`.
    function _convert(uint256 cost) internal {
        _deposit(address(module));
        vm.prank(custodian);
        module.itemArrived(cost);
    }

    /// Rent frog `i` (32..63 are eligible) for `renter`.
    function _rent(uint256 i, address who) internal returns (uint256 rentalId, uint256 frogId) {
        bytes memory att = abi.encode(frogIds[i], Merkle.proof(frogLeaves, i), keccak256(abi.encode("rent tx", i, nextTx++)), uint32(0));
        vm.prank(custodian);
        rentalId = module.rentIn(who, bytes(TAPROOT), Merkle.proof(rentLeaves, i - 32), att);
        frogId = module.rental(rentalId).itemId;
    }

    /// (W1) and (M2): the wrapper's ledger with ETH-backed units, and the module holding every wei it owes.
    function _books() internal view {
        assertEq(wrapper.totalSupply(), (wrapper.held() - wrapper.openTickets()) * ONE + wrapper.ethBacked(),
            "supply == (held - open) x ONE + ethBacked");
        assertGe(address(module).balance, module.backing() + module.reserve() + module.surplus() + module.revenue() + module.gradEth()
            + module.rentalEscrow() + module.renterPool() + module.owedTotal(), "the module holds every wei it owes");
        assertGe(wrapper.balanceOf(address(module)), module.gradUnits() + module.floatUnits(), "the module holds the graduations' and the floats' units");
        if (wrapper.ethBacked() == 0) assertEq(module.backing(), 0, "no backing without ETH-backed units");
        assertLe(module.reserveUnits(), wrapper.ethBacked(), "reserve units are ETH-backed units");
    }
}
