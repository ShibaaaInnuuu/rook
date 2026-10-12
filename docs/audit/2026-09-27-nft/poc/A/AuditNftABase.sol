// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Base} from "../utils/Base.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";
import {PosterSource} from "../../src/floor/PosterSource.sol";
import {EthNftWrapper, IListingPause} from "../../src/nft/EthNftWrapper.sol";
import {IKeeperSource} from "../../src/interfaces/IKeeperSource.sol";
import {IFloorWrapped} from "../../src/interfaces/IFloorWrapped.sol";
import {IRandomnessBeacon} from "../../src/interfaces/IRandomnessBeacon.sol";
import {ICapSource} from "../../src/interfaces/ICapSource.sol";
import {ILiquiditySeeder} from "../../src/interfaces/ILiquiditySeeder.sol";

/// A stand-in CurveFactory: enough of ICapSource for FloorModule.wire and the graduation lane's `_onlyCurve`.
contract AuditNftAFactory is ICapSource {
    address public immutable wrapped;
    address public immutable curveRouter;
    constructor(address w, address r) { wrapped = w; curveRouter = r; }
    function freshCap() external pure returns (uint256, bool) { return (0, false); }
    function routerOf(address) external view returns (address) { return curveRouter; }
    function seeder() external pure returns (ILiquiditySeeder) { return ILiquiditySeeder(address(0)); }
}

/// Audit 2026-09-27 (nft-side, reviewer A): the Milady collection exactly as DeployStack builds it at STAGE 0 —
/// `StackChecks.defaultParams(1e18, 0)` (reserve 25 %, maxItemsPerDay 3, 5 units/h each way, public cap 10, maxEthBacked 40),
/// `EthNftWrapper.initMintLimit(3)` (stage 0 mintsPerDay), the module's keeper read through a `PosterSource` naming the canister.
/// The deployer of the wrapper is this test contract (the deploy EOA in DeployStack).
abstract contract AuditNftABase is Base {
    EthNftWrapper w;
    PosterSource poster;
    FloorModule module;
    AuditNftAFactory factory;
    address collection = makeAddr("Milady on chain 1");
    address payee;                               // DeployStack: ETCH_PAYEE defaults to the custodian
    address platform = makeAddr("platform (the treasury)");
    address curve = makeAddr("a coin's curve router");
    address coin = makeAddr("a coin");
    address alice = makeAddr("alice (an ordinary holder)");
    address bob = makeAddr("bob (an ordinary redeemer)");
    address mallory = makeAddr("mallory");
    address guardian = makeAddr("guardian");
    uint64 constant ETHEREUM = 1;
    uint256 constant ONE = 1e18;
    uint256 constant F = 0.93 ether;             // Milady's measured floor, docs/v7/phase1-nft.md §1.1
    uint256 constant STAGE0_MINTS = 3;

    function _params() internal pure returns (FloorModule.Params memory p) {
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
        p.maxItemsPerDay = 3;                    // StackChecks.stageOf(0).itemsPerDay
        p.reserveBps = 2_500;
        p.fillBandBps = 300;
    }

    function setUp() public virtual override {
        super.setUp();
        payee = custodian;
        w = new EthNftWrapper(IListingPause(address(registry)), 0, IRandomnessBeacon(address(beacon)), ETHEREUM, collection, custodian,
            "Wrapped Milady", "wMIL", PBS);
        poster = new PosterSource(IKeeperSource(address(registry)), custodian);
        module = new FloorModule(IFloorWrapped(address(w)), IKeeperSource(address(poster)), payee, platform, address(0), _params(), PBS);
        w.wireFloorModule(address(module));
        w.initMintLimit(STAGE0_MINTS);
        factory = new AuditNftAFactory(address(w), curve);
        module.wire(factory);
        vm.prank(owner);
        registry.setGuardian(guardian);
        _post(F);
    }

    function _skip(uint256 secs) internal {
        vm.warp(vm.getBlockTimestamp() + secs);
        vm.roll(vm.getBlockNumber() + secs / PBS + 1);
    }

    function _post(uint256 price) internal {
        vm.prank(custodian);
        module.postFloor(price, 10_000, 50);
    }

    /// The canister re-posts the same price every 30 minutes (its cadence, docs §4.2) for `hours_` hours: the volatility term decays.
    function _holdFor(uint256 price, uint256 halfHours) internal {
        for (uint256 i = 0; i < halfHours; ++i) { _skip(30 minutes); _post(price); }
    }

    /// A curve graduates `eth` into the module (FrogCurve.graduate → enqueue; filled at once while fresh) and takes its units.
    function _graduate(uint256 eth) internal returns (uint256 units) {
        address c = address(uint160(uint256(keccak256(abi.encode("coin", eth, vm.getBlockTimestamp())))));
        vm.deal(curve, curve.balance + eth);
        vm.prank(curve);
        module.enqueue{value: eth}(c, 0, 0);
        vm.prank(curve);
        (units,,) = module.release(c);
    }

    function _tx(uint256 tokenId, string memory what) internal pure returns (bytes32) { return keccak256(abi.encode(what, tokenId)); }

    function _deposit(uint256 tokenId, address to) internal returns (uint256 itemId) {
        vm.prank(custodian);
        itemId = w.mintDeposit(tokenId, _tx(tokenId, "EWN1 safeTransferFrom"), to);
    }

    function _mintPurchase(uint256 tokenId, address to) internal returns (uint256 itemId) {
        vm.prank(custodian);
        itemId = w.mint(tokenId, _tx(tokenId, "seaport fill"), to);
    }

    function _burn(address who, uint256 units) internal returns (uint256 eth) {
        vm.startPrank(who);
        w.approve(address(module), units);
        eth = module.burn(units, 0, who);
        vm.stopPrank();
    }

    function _resolve(uint256 id) internal {
        w.resolve(id, abi.encodePacked(keccak256(abi.encode("drand", id))));
    }

    function _books() internal view {
        assertEq(w.totalSupply(), (w.held() - w.openTickets()) * ONE + w.ethBacked(), "supply == (held - open) x ONE + ethBacked");
        assertGe(address(module).balance, module.backing() + module.reserve() + module.surplus() + module.revenue() + module.gradEth()
            + module.rentalEscrow() + module.renterPool() + module.owedTotal(), "the module holds every wei it owes");
        assertEq(module.netUnits() + module.reserveUnits() + module.transitUnits(), w.ethBacked(), "C + R + T = E");
    }
}
