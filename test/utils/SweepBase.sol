// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Base} from "./Base.sol";
import {Merkle} from "./Merkle.sol";
import {Wrapper} from "../../src/Wrapper.sol";
import {SweepVault} from "../../src/SweepVault.sol";
import {MintDesk} from "../../src/curve/MintDesk.sol";

/// A venue standing in for the wFROG/ETH pool's router. It sells the wFROG it holds at whatever price the caller's `swapData`
/// names, and has a few ways to misbehave, each a separate function so a test names the one it means.
contract MockVenue {
    Wrapper public immutable w;
    constructor(Wrapper w_) { w = w_; }
    receive() external payable {}

    /// Sell `msg.value` worth at `weiPerWhole`, to the caller.
    function buy(uint256 weiPerWhole) external payable {
        w.transfer(msg.sender, (msg.value * 1e18) / weiPerWhole);
    }
    /// Use only `use` of what was sent, refund the rest.
    function buyAndRefund(uint256 weiPerWhole, uint256 use) external payable {
        w.transfer(msg.sender, (use * 1e18) / weiPerWhole);
        (bool ok,) = msg.sender.call{value: msg.value - use}("");
        require(ok, "refund refused");
    }
    /// Sell to somebody else: the vault pays and receives nothing.
    function buyFor(uint256 weiPerWhole, address to) external payable {
        w.transfer(to, (msg.value * 1e18) / weiPerWhole);
    }
    /// Take the ETH, send nothing.
    function keep() external payable {}
    /// Send back more than it was sent.
    function overRefund() external payable {
        (bool ok,) = msg.sender.call{value: msg.value + 1}("");
        require(ok, "refused");
    }
    /// Try to pull the caller's wFROG.
    function steal(uint256 amount) external payable {
        w.transferFrom(msg.sender, address(this), amount);
    }
    /// Try to re-enter the vault mid-sweep.
    function reenter(address coin) external payable {
        SweepVault(payable(msg.sender)).sweep(coin, 1, "", 1);
    }
}

/// `Base`, plus a second inscriptions listing of 64 frogs (id 2) with a wrapper over it, and two sweep listings standing in
/// for what the registry branch will create (`ISweepListing`): id 3 has the RAFFLE vault, id 4 the BURN vault. The registry
/// here cannot deploy a SweepVault as a listing's vault (ListingRegistry.sol is not edited on this branch), so ids 3 and 4 are
/// ordinary listings whose pause flag the vaults read, and the mock launcher routes the sweep coins to them.
abstract contract SweepBase is Base {
    Wrapper wrapper;
    SweepVault raffle;
    SweepVault burn;
    MockVenue venue;
    MintDesk desk;
    address payee = makeAddr("payee (the canister's ETH address)");

    uint256 constant FROGS = 2;
    uint256 constant RAFFLE_LISTING = 3;
    uint256 constant BURN_LISTING = 4;
    uint256 constant ONE = 1e18;
    uint256 constant N_FROGS = 64;
    uint256 constant PRICE = 0.05 ether;           // the venue's price per whole wFROG in most tests
    uint256 constant SUITE_CEILING = 1_000_000 ether;
    string constant TAPROOT = "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0";
    address distribution = makeAddr("distribution");

    address coinR = makeAddr("coinR (raffle)");
    address coinR2 = makeAddr("coinR2 (raffle)");
    address coinX = makeAddr("coinX (burn)");
    address routerR = makeAddr("routerR");
    address routerR2 = makeAddr("routerR2");
    address routerX = makeAddr("routerX");

    string[] frogIds;
    bytes32[] frogLeaves;
    uint256 nextFrog;
    uint256 nextTx;

    function setUp() public virtual override {
        super.setUp();
        for (uint256 i = 0; i < N_FROGS; ++i) {
            frogIds.push(string.concat(vm.toString(keccak256(abi.encode("frog", i))), "i0"));
            frogLeaves.push(registry.leafOf(frogIds[i]));
        }
        vm.startPrank(owner);
        assertEq(registry.listInscriptions("Frogs", "frogs", Merkle.root(frogLeaves), CUSTODY_BTC), FROGS);
        assertEq(registry.listInscriptions("Frogs raffle", "frogs-raffle", bytes32(uint256(1)), "bc1p-unused"), RAFFLE_LISTING);
        assertEq(registry.listInscriptions("Frogs burn", "frogs-burn", bytes32(uint256(1)), "bc1p-unused"), BURN_LISTING);
        vm.stopPrank();
        wrapper = new Wrapper(registry, distributor, beacon, FROGS, distribution, custodian, CUSTODY_BTC, "Wrapped Frog", "wFROG", PBS);
        wrapper.initMintLimit(1_000);   // the suites mint many frogs a day; H11's own tests use the default
        venue = new MockVenue(wrapper);
        desk = new MintDesk(wrapper, payee, 6 hours, PBS);
        raffle = new SweepVault(registry, RAFFLE_LISTING, wrapper, SweepVault.Policy.RAFFLE, address(venue), launcher, desk, PBS);
        burn = new SweepVault(registry, BURN_LISTING, wrapper, SweepVault.Policy.BURN, address(venue), launcher, desk, PBS);
        // C-01: a vault takes no cap without a ceiling; the owner's first applies at once. The suites about something else get a
        // high one (test/floor/CapStep.t.sol tests the ceiling itself)
        vm.startPrank(owner);
        raffle.proposeCapCeiling(SUITE_CEILING);
        burn.proposeCapCeiling(SUITE_CEILING);
        vm.stopPrank();
        launcher.set(coinR, routerR, creatorA, RAFFLE_LISTING);
        launcher.set(coinR2, routerR2, creatorB, RAFFLE_LISTING);
        launcher.set(coinX, routerX, creatorA, BURN_LISTING);
        vm.deal(routerR, 1_000 ether);
        vm.deal(routerR2, 1_000 ether);
        vm.deal(routerX, 1_000 ether);
    }

    /// A holder deposits the next frog; its wFROG goes to `to`.
    function _deposit(address to) internal returns (uint256 frogId) {
        uint256 i = nextFrog++;
        string memory id = frogIds[i];
        bytes32[] memory proof = Merkle.proof(frogLeaves, i);
        bytes32 txid = keccak256(abi.encode("deposit tx", nextTx++));
        vm.prank(custodian);
        frogId = wrapper.wrapDeposit(id, proof, txid, 0, to);
    }

    /// Stock the venue with `n` whole wFROG.
    function _stockVenue(uint256 n) internal {
        for (uint256 i = 0; i < n; ++i) _deposit(address(venue));
    }

    function _fundCoin(SweepVault v, address coin, address router_, uint256 amount) internal {
        vm.prank(router_);
        v.deposit{value: amount}(coin);
    }

    /// The keeper puts `weiPerWrapped` on `v`. H12 bounds a post while the last cap is fresh (±20%, ten minutes apart); the
    /// suites that are about something else post whatever they need, so this helper lets a fresh cap lapse first when the new one
    /// is not a legal step (test/floor/CapStep.t.sol is where the bound itself is tested).
    function _postCap(SweepVault v, uint256 weiPerWrapped) internal {
        // a post may rise at most one step (±20%) above the last cap, fresh or stale (C-01): climb there in legal steps
        while (v.cap() != 0 && weiPerWrapped * 10 > v.cap() * 12) {
            _capGap(v);
            uint256 up = v.cap() + (v.cap() * 2_000) / 10_000;
            vm.prank(keeper);
            v.postCap(up);
        }
        if (v.capFresh()) {
            uint256 c = v.cap();
            bool soon = vm.getBlockTimestamp() < uint256(v.capAt()) + 10 minutes
                || vm.getBlockNumber() < uint256(v.capAtBlock()) + 10 minutes / PBS;
            bool jump = weiPerWrapped * 10 > c * 12 || weiPerWrapped * 10 < c * 8;
            if (soon || jump) {
                vm.warp(uint256(v.capAt()) + 1 hours);
                vm.roll(uint256(v.capAtBlock()) + 1 hours / PBS + 1);
            }
        }
        vm.prank(keeper);
        v.postCap(weiPerWrapped);
    }

    /// C-01: the owner's first ceiling (at once), high enough for the suites about something else.
    function _ceiling(SweepVault v) internal {
        vm.prank(owner);
        v.proposeCapCeiling(SUITE_CEILING);
    }

    /// Ten minutes since the last cap on both clocks, if it is fresh (H12's gap).
    function _capGap(SweepVault v) internal {
        if (!v.capFresh()) return;
        if (vm.getBlockTimestamp() < uint256(v.capAt()) + 10 minutes) vm.warp(uint256(v.capAt()) + 10 minutes);
        if (vm.getBlockNumber() < uint256(v.capAtBlock()) + 10 minutes / PBS) vm.roll(uint256(v.capAtBlock()) + 10 minutes / PBS);
    }

    /// The keeper's draw to Open: announce (the seed round pinned, a whole unit reserved), then open with the seed's (mock)
    /// signature and the published tree: `leaves` then the sentinel `leaf(0, total, snapshotAt)` (C-03). Claims prove against
    /// `tree`; a leaf keeps its index.
    function _openDraw(SweepVault v, address coin, bytes32[] memory leaves, uint256 total) internal returns (uint256 id, bytes32[] memory tree) {
        vm.prank(keeper);
        id = v.announceDraw(coin);
        // a carried seed round (review2 N-2) may already be proven: drand has one signature per round, so use its randomness
        bytes32 seed = v.beacon().randomnessOf(v.draw(id).seedRound);
        if (seed == bytes32(0)) seed = keccak256(abi.encode("seed", address(v), id));
        tree = new bytes32[](leaves.length + 1);
        for (uint256 i = 0; i < leaves.length; ++i) tree[i] = leaves[i];
        tree[leaves.length] = v.leaf(address(0), total, v.snapshotTimeOf(id, seed));
        vm.prank(keeper);
        v.openDraw(id, Merkle.root(tree), total, "ipfs://tickets", Merkle.proof(tree, leaves.length), abi.encodePacked(seed));
    }

    /// `_openDraw` with a single holder's range as the list.
    function _openDrawOne(SweepVault v, address coin, address holder, uint256 total) internal returns (uint256 id, bytes32[] memory tree) {
        bytes32[] memory leaves = new bytes32[](1);
        leaves[0] = v.leaf(holder, 0, total);
        return _openDraw(v, coin, leaves, total);
    }

    /// The honest path: fund `coin`, post a cap at the venue price, sweep `n` whole wFROG.
    function _sweepWhole(SweepVault v, address coin, address router_, uint256 n) internal {
        _fundCoin(v, coin, router_, n * PRICE);
        _postCap(v, PRICE);
        v.sweep(coin, n * PRICE, abi.encodeCall(MockVenue.buy, (PRICE)), n * ONE);
    }

    function _resolveTicket(uint256 id, bytes32 r) internal {
        if (r == bytes32(0)) r = bytes32(uint256(1));
        wrapper.resolve(id, abi.encodePacked(r));
    }

    function _assertLedger() internal view {
        assertEq(wrapper.totalSupply(), (wrapper.held() - wrapper.openTickets()) * ONE, "supply == (held - open) x ONE");
    }

    function _assertVaultBooks(SweepVault v) internal view {
        assertGe(wrapper.balanceOf(address(v)), v.totalUnits() + v.totalReserved(), "wFROG covers units + reserved");
        assertGe(address(v).balance, v.totalCredit(), "ETH covers credit");
    }
}
