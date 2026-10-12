// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {ListingRegistry} from "../../src/ListingRegistry.sol";
import {AssetVault} from "../../src/AssetVault.sol";
import {Distributor} from "../../src/Distributor.sol";
import {MockLauncher, MockBeacon} from "./Mocks.sol";
import {Merkle} from "./Merkle.sol";
import {Clock} from "./Clock.sol";

/// One registry with an inscriptions listing (id 0) and a rune listing (id 1, divisibility 2), a mock launcher in which
/// `router` is the router of `coinA` and `coinB`, and the three roles named.
abstract contract Base is Test {
    /// The parent chain's block time the suite deploys with: 4663's (src/lib/Wait.sol). Deployment config, not a constant.
    uint256 constant PBS = 12;
    ListingRegistry registry;
    MockLauncher launcher;
    MockBeacon beacon;
    Distributor distributor;
    AssetVault ordVault;
    AssetVault runeVault;

    address owner = makeAddr("owner");
    /// the inscriptions listing's delivery address, and the wrapper's custody address: BIP-86's first test vector
    string constant CUSTODY_BTC = "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr";
    address custodian = makeAddr("custodian (the canister's EVM address)");
    address treasury = makeAddr("treasury");
    address keeper = makeAddr("keeper");
    address purchaser = makeAddr("purchaser");
    address attestor = makeAddr("attestor");
    address router = makeAddr("router");
    address coinA = makeAddr("coinA");
    address coinB = makeAddr("coinB");
    address creatorA = makeAddr("creatorA");
    address creatorB = makeAddr("creatorB");
    address stranger = makeAddr("stranger");

    string[] ids;
    bytes32[] idLeaves;

    function setUp() public virtual {
        // docs/phase1.md F4: the bond requirement starts at 100% and can only be LOWERED after seven public days. The ordinary tests
        // are about everything else, so they lower it to zero the slow, legitimate way; test/Bond.t.sol raises it back (at once).
        vm.warp(1_800_000_000 - 8 days);
        vm.roll(26_000_000);                 // a parent-chain height near the measured one, so both clocks start somewhere real
        ids.push("aaaa1111i0"); ids.push("bbbb2222i0"); ids.push("cccc3333i0"); ids.push("dddd4444i0");
        vm.startPrank(owner);
        registry = new ListingRegistry(owner, treasury);
        registry.setBondBps(0);
        Clock.skip(8 days);                  // both clocks, or the seven-day delay below never elapses
        registry.acceptBondBps();
        for (uint256 i = 0; i < ids.length; ++i) idLeaves.push(registry.leafOf(ids[i]));
        launcher = new MockLauncher();
        beacon = new MockBeacon();
        distributor = new Distributor(registry, beacon, PBS);
        registry.wire(address(launcher), address(distributor));
        registry.listInscriptions("Test Collection", "test-collection", Merkle.root(idLeaves), CUSTODY_BTC);
        registry.listRune("TEST.RUNE", "840000:1", 2, "bc1p-rune-vault");
        registry.proposeRoles(keeper, purchaser, attestor);
        vm.stopPrank();
        ordVault = AssetVault(registry.vaultOf(0));
        runeVault = AssetVault(registry.vaultOf(1));
        launcher.set(coinA, router, creatorA, 0);
        launcher.set(coinB, router, creatorB, 0);
        vm.deal(router, 1_000 ether);
    }

    function _fund(AssetVault v, address coin, uint256 amount) internal {
        vm.prank(router);
        v.deposit{value: amount}(coin);
    }

    function _proofOf(uint256 i) internal view returns (bytes32[] memory) {
        return Merkle.proof(idLeaves, i);
    }
}
