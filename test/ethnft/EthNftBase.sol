// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Base} from "../utils/Base.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";
import {PosterSource} from "../../src/floor/PosterSource.sol";
import {EthNftWrapper, IListingPause} from "../../src/nft/EthNftWrapper.sol";
import {IKeeperSource} from "../../src/interfaces/IKeeperSource.sol";
import {IFloorWrapped} from "../../src/interfaces/IFloorWrapped.sol";
import {IRandomnessBeacon} from "../../src/interfaces/IRandomnessBeacon.sol";

/// docs/v7/phase1-nft.md §3: an Ethereum collection (Milady, chain 1) on 4663. `Base`'s registry (listing 0's pause flag gates the
/// wrapper's mints), an `EthNftWrapper` whose custodian is the canister's EVM address, a `PosterSource` naming the same address as
/// the module's keeper, and a `FloorModule` built on them with the frog defaults (reserve share 0 here; the invariant suite runs
/// 25 %). The canister posts a 0.93 ETH floor. The canister is a plain address: what it attests on Ethereum is its word here.
abstract contract EthNftBase is Base {
    EthNftWrapper w;
    PosterSource poster;
    FloorModule module;
    address collection = makeAddr("Milady on chain 1");
    address payee = makeAddr("payee (the canister's EVM address on 4663)");
    address platform = makeAddr("platform");
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    address guardian = makeAddr("guardian");
    uint64 constant ETHEREUM = 1;
    uint256 constant ONE = 1e18;
    uint256 constant F = 0.93 ether;
    address constant DEAD = 0x000000000000000000000000000000000000dEaD;
    uint16 reserveBps;

    function _params() internal view virtual returns (FloorModule.Params memory p) {
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
        p.reserveBps = reserveBps;
        p.fillBandBps = 300;
    }

    function setUp() public virtual override {
        super.setUp();
        Base_skipRoles();
        w = new EthNftWrapper(IListingPause(address(registry)), 0, IRandomnessBeacon(address(beacon)), ETHEREUM, collection, custodian,
            "Wrapped Milady", "wMIL", PBS);
        w.initMintLimit(1_000);
        poster = new PosterSource(IKeeperSource(address(registry)), custodian);
        module = new FloorModule(IFloorWrapped(address(w)), IKeeperSource(address(poster)), payee, platform, address(0), _params(), PBS);
        w.wireFloorModule(address(module));
        vm.prank(owner);
        registry.setGuardian(guardian);
        _post(F);
    }

    /// `Base` proposes the roles; they apply at once (the first roles), so the registry's keeper is the keeper EOA.
    function Base_skipRoles() internal view {
        require(registry.keeper() == keeper, "the registry's keeper is the keeper EOA");
    }

    function _skip(uint256 secs) internal {
        vm.warp(vm.getBlockTimestamp() + secs);
        vm.roll(vm.getBlockNumber() + secs / PBS + 1);
    }

    function _post(uint256 price) internal {
        vm.prank(custodian);
        module.postFloor(price, 10_000, 50);
    }

    function _mintAtModule(address who, uint256 eth) internal returns (uint256 units) {
        vm.deal(who, who.balance + eth);
        vm.prank(who);
        units = module.mint{value: eth}(who, 0);
    }

    function _burnAtModule(address who, uint256 units) internal returns (uint256 eth) {
        vm.startPrank(who);
        w.approve(address(module), units);
        eth = module.burn(units, 0, who);
        vm.stopPrank();
    }

    function _tx(uint256 tokenId) internal pure returns (bytes32) { return keccak256(abi.encode("seaport fill on Ethereum", tokenId)); }

    /// The canister mints token `tokenId` (bought by `_tx(tokenId)`) to `to`.
    function _mintItem(uint256 tokenId, address to) internal returns (uint256 itemId) {
        vm.prank(custodian);
        itemId = w.mint(tokenId, _tx(tokenId), to);
    }

    /// The whole Phase-1 buy on the 4663 side: the module advances, (Relay, Seaport and finality happen on Ethereum), the canister
    /// mints the unit to the module and settles at `cost`.
    function _buyOne(uint256 tokenId, uint256 cost) internal returns (uint256 itemId) {
        uint256 amount = module.maxItemCost();
        vm.prank(custodian);
        module.advance(amount);
        itemId = _mintItem(tokenId, address(module));
        vm.prank(custodian);
        module.settleAdvance(cost > amount ? amount : cost);
    }

    function _redeem(address who, address to) internal returns (uint256 id) {
        vm.prank(who);
        id = w.redeem(to);
    }

    function _resolve(uint256 id, bytes32 r) internal {
        if (r == bytes32(0)) r = bytes32(uint256(1));
        w.resolve(id, abi.encodePacked(r));
    }

    function _deliver(uint256 id) internal {
        vm.prank(custodian);
        w.recordDelivery(id, keccak256(abi.encode("transferFrom on Ethereum", id)));
    }

    function _tokenOf(uint256 ticketId) internal view returns (uint256) {
        return w.item(w.ticket(ticketId).itemId).tokenId;
    }

    /// The wrapper's ledger and the module's books.
    function _books() internal view {
        assertEq(w.totalSupply(), (w.held() - w.openTickets()) * ONE + w.ethBacked(), "supply == (held - open) x ONE + ethBacked");
        assertGe(address(module).balance, module.backing() + module.reserve() + module.surplus() + module.revenue() + module.gradEth()
            + module.rentalEscrow() + module.renterPool() + module.owedTotal(), "the module holds every wei it owes");
        assertEq(module.netUnits() + module.reserveUnits() + module.transitUnits(), w.ethBacked(), "C + R + T = E");
    }
}
