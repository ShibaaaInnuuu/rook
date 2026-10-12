// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Base} from "../utils/Base.sol";
import {MockEscrow, MockCurve} from "../utils/Mocks.sol";
import {Clones} from "@openzeppelin/contracts/proxy/Clones.sol";
import {AssetVault} from "../../src/AssetVault.sol";
import {Distributor} from "../../src/Distributor.sol";
import {FeeRouter} from "../../src/FeeRouter.sol";
import {IPonsV2FeeEscrow, IPonsV2BondingCurve, IPonsV2MemeHook} from "../../src/interfaces/IPons.sol";
import {Clock} from "../utils/Clock.sol";

/// docs/security/gas.md is this file's output. Sizes: 1, 10, 32 and MAX_PAYERS (64) paying coins — 100 cannot happen, by H1; Merkle depth 10, 16, 20.
contract GasTest is Base {
    bytes32 constant TXID = bytes32(uint256(7));
    uint256 constant BLOCK = 32_000_000;

    function _coins(uint256 n, AssetVault v, uint256 each) internal {
        for (uint256 i = 0; i < n; ++i) {
            address c = address(uint160(0xC0FFEE00 + i));
            launcher.set(c, router, creatorA, v == runeVault ? 1 : 0);
            _fund(v, c, each);
        }
    }

    function _payers(uint256 n) internal returns (uint256 rel, uint256 ret, uint256 wo, uint256 settleRune) {
        uint256 snap = vm.snapshotState();
        _coins(n, runeVault, 0.01 ether);
        uint256 maxEth = n * 0.01 ether;
        vm.prank(keeper); runeVault.postCap(maxEth);
        vm.prank(keeper); uint256 id = runeVault.declarePurchase(maxEth, 100, "ref");
        Clock.skip(1 hours);
        uint256 g = gasleft(); runeVault.release(id); rel = g - gasleft();
        uint256 mid = vm.snapshotState();
        vm.deal(purchaser, maxEth);
        g = gasleft(); vm.prank(purchaser); runeVault.returnUnspent{value: maxEth / 2}(id); ret = g - gasleft();
        vm.revertToState(mid);
        g = gasleft(); vm.prank(attestor); runeVault.settleRune(id, 1_000_000, TXID, 1); settleRune = g - gasleft();
        vm.revertToState(mid);
        vm.prank(owner); registry.setBondBps(10_000);
        vm.deal(address(this), maxEth); runeVault.postBond{value: maxEth}();
        Clock.skip(4 days);
        g = gasleft(); vm.prank(owner); runeVault.writeOff(id, "x"); wo = g - gasleft();
        vm.revertToState(snap);
    }

    function test_gas_loopsByPayingCoins() public {
        uint256[4] memory sizes = [uint256(1), 10, 32, runeVault.MAX_PAYERS()];
        for (uint256 k = 0; k < 4; ++k) {
            (uint256 rel, uint256 ret, uint256 wo, uint256 sr) = _payers(sizes[k]);
            emit log_named_uint(string.concat("release, payers=", vm.toString(sizes[k])), rel);
            emit log_named_uint(string.concat("returnUnspent, payers=", vm.toString(sizes[k])), ret);
            emit log_named_uint(string.concat("settleRune, payers=", vm.toString(sizes[k])), sr);
            emit log_named_uint(string.concat("writeOff (bond slashed to every payer), payers=", vm.toString(sizes[k])), wo);
            assertLt(rel, BLOCK / 4); assertLt(ret, BLOCK / 2); assertLt(wo, BLOCK / 2); assertLt(sr, BLOCK / 2);
        }
    }

    function _tree(uint256 depth, bytes32 leaf) internal pure returns (bytes32 root, bytes32[] memory proof) {
        proof = new bytes32[](depth);
        root = leaf;
        for (uint256 i = 0; i < depth; ++i) {
            bytes32 sib = keccak256(abi.encode("sibling", i));
            proof[i] = sib;
            root = root < sib ? keccak256(abi.encodePacked(root, sib)) : keccak256(abi.encodePacked(sib, root));
        }
    }

    function test_gas_merkleByDepth() public {
        launcher.set(coinA, router, creatorA, 1);
        uint256[3] memory depths = [uint256(10), 16, 20];
        for (uint256 k = 0; k < 3; ++k) {
            uint256 snap = vm.snapshotState();
            _fund(runeVault, coinA, 1 ether);
            vm.prank(keeper); runeVault.postCap(1 ether);
            vm.prank(keeper); uint256 id = runeVault.declarePurchase(1 ether, 100, "f");
            Clock.skip(1 hours);
            runeVault.release(id);
            vm.prank(attestor); runeVault.settleRune(id, 100_000, TXID, 1);
            address holder = address(0xA11CE);
            (bytes32 root, bytes32[] memory proof) = _tree(depths[k], distributor.shareLeaf(holder, 500));
            vm.prank(creatorA); distributor.choose(1, 0, Distributor.Mode.ProRata, address(0));
            vm.prank(keeper); distributor.openDistribution(1, 0, root, "ipfs://s");
            Clock.skip(15 minutes);
            uint256 g = gasleft(); distributor.claimShare(1, 0, holder, 500, proof); uint256 used = g - gasleft();
            emit log_named_uint(string.concat("claimShare, depth=", vm.toString(depths[k]), " (2^depth holders)"), used);
            assertLt(used, 200_000);
            vm.revertToState(snap);
        }
    }

    function test_gas_everydayCalls() public {
        MockEscrow escrow = new MockEscrow();
        MockCurve curve = new MockCurve(escrow);
        FeeRouter r = FeeRouter(payable(Clones.clone(address(new FeeRouter()))));
        r.initialize(coinA, registry, ordVault, IPonsV2FeeEscrow(address(escrow)), IPonsV2BondingCurve(address(curve)), IPonsV2MemeHook(address(0xB00C)));
        curve.setRecipient(address(r));
        launcher.set(coinA, address(r), creatorA, 0);
        vm.deal(address(this), 10 ether);
        escrow.credit{value: 1 ether}(address(r));
        uint256 g = gasleft(); r.harvest(); emit log_named_uint("harvest, first for a coin (queues it)", g - gasleft());
        escrow.credit{value: 1 ether}(address(r));
        g = gasleft(); r.harvest(); emit log_named_uint("harvest, a coin already queued", g - gasleft());
        vm.startPrank(keeper);
        g = gasleft(); ordVault.postCap(1 ether); emit log_named_uint("postCap", g - gasleft());
        g = gasleft(); uint256 id = ordVault.declarePurchase(1 ether, 1, "https://example/cap"); emit log_named_uint("declarePurchase", g - gasleft());
        vm.stopPrank();
        Clock.skip(1 hours);
        g = gasleft(); ordVault.release(id); emit log_named_uint("release, one payer", g - gasleft());
        vm.prank(attestor);
        g = gasleft(); uint256 aid = ordVault.settleInscription(id, ids[0], TXID, 1, _proofOf(0)); emit log_named_uint("settleInscription (closes the purchase)", g - gasleft());
        vm.prank(creatorA);
        g = gasleft(); distributor.choose(0, aid, Distributor.Mode.DrawWeighted, address(0)); emit log_named_uint("choose", g - gasleft());
        (bytes32 root, bytes32[] memory proof) = _tree(16, distributor.ticketLeaf(address(0xA11CE), 0, 1_000_000));
        vm.prank(keeper);
        g = gasleft(); distributor.openDraw(0, aid, root, 1_000_000, "ipfs://tickets"); emit log_named_uint("openDraw", g - gasleft());
        Clock.skip(15 minutes);
        g = gasleft(); distributor.requestDraw(0, aid); emit log_named_uint("requestDraw", g - gasleft());
        g = gasleft(); distributor.resolve(0, aid, abi.encodePacked(bytes32(uint256(5)))); emit log_named_uint("resolve (mock beacon; add ~143k for the real verifier)", g - gasleft());
        g = gasleft(); distributor.claimWin(0, aid, address(0xA11CE), 0, 1_000_000, proof); emit log_named_uint("claimWin, depth 16", g - gasleft());
        vm.prank(address(0xA11CE));
        g = gasleft(); distributor.setDestination(0, aid, "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0"); emit log_named_uint("setDestination (bech32m checked on chain)", g - gasleft());
    }
}
