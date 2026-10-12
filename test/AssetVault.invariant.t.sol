// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Base} from "./utils/Base.sol";
import {AssetVault} from "../src/AssetVault.sol";
import {Clock} from "./utils/Clock.sol";

/// Every actor, with every key, doing anything in any order — including the owner, the keeper and the purchaser trying to take money.
contract VaultHandler is Test {
    AssetVault public vault;
    address owner; address keeper; address purchaser; address attestor; address router; address[2] coins;
    address treasury;

    uint256 public ghostReleased;           // sum of maxEth over releases that succeeded
    uint256 public ghostReturned;
    uint256 public ghostDeposited;
    uint256 public ghostBonded;             // docs/phase1.md F4: ETH posted as bond, by somebody who is not the purchaser
    mapping(uint256 => uint256) public boundAtDeclaration;
    mapping(uint256 => uint64) public declaredAt;
    uint256 public violations;

    constructor(AssetVault v, address o, address k, address p, address a, address r, address c0, address c1, address t) {
        vault = v; owner = o; keeper = k; purchaser = p; attestor = a; router = r; coins = [c0, c1]; treasury = t;
    }

    function deposit(uint96 amount, bool which) external {
        amount = uint96(bound(amount, 1, 50 ether));
        vm.deal(router, amount);
        vm.prank(router);
        vault.deposit{value: amount}(coins[which ? 1 : 0]);
        ghostDeposited += amount;
    }

    /// docs/token/protocol-token-v1.md §2. The treasury's share arrives by the other door. It is the same accounting, so the same
    /// invariants must hold: the books still match the balance and nothing new leaves.
    function depositFromTreasury(uint96 amount, bool which) external {
        amount = uint96(bound(amount, 1, 50 ether));
        vm.deal(treasury, amount);
        vm.prank(treasury);
        vault.depositFrom{value: amount}(coins[which ? 1 : 0]);
        ghostDeposited += amount;
    }

    /// A bond from a third party, so the purchaser's balance stays exactly what was released. A write-off then slashes it to the coins.
    function postBond(uint96 amount) external {
        amount = uint96(bound(amount, 1, 20 ether));
        address bondsman = address(0xB07D);
        vm.deal(bondsman, amount);
        vm.prank(bondsman);
        vault.postBond{value: amount}();
        ghostBonded += amount;
    }

    function postCap(uint96 cap) external {
        vm.prank(keeper);
        vault.postCap(bound(cap, 1, 5 ether));
    }

    function declare(uint96 maxEth, uint8 q) external {
        uint256 quantity = bound(q, 1, 5) * 100;      // whole runes at divisibility 2, so the cap is per unit of these
        uint256 id = vault.purchaseCount();
        uint256 boundNow = vault.boundFor(quantity);
        vm.prank(keeper);
        try vault.declarePurchase(bound(maxEth, 1, 30 ether), quantity, "h") {
            boundAtDeclaration[id] = boundNow;
            declaredAt[id] = uint64(vm.getBlockTimestamp());
            if (vault.purchase(id).maxEth > boundNow) violations++;
        } catch {}
    }

    function release(uint256 id) external {
        uint256 n = vault.purchaseCount();
        if (n == 0) return;
        id = id % n;
        uint256 before = purchaser.balance;
        try vault.release(id) {
            AssetVault.Purchase memory p = vault.purchase(id);
            ghostReleased += p.maxEth;
            if (purchaser.balance - before != p.maxEth) violations++;
            if (vm.getBlockTimestamp() < declaredAt[id] + 1 hours) violations++;      // never before the public hour
            if (p.maxEth > boundAtDeclaration[id]) violations++;
        } catch {}
    }

    function cancel(uint256 id) external {
        uint256 n = vault.purchaseCount();
        if (n == 0) return;
        vm.prank(owner);
        try vault.cancel(id % n) {} catch {}
    }

    function settle(uint256 id, uint64 amount) external {
        uint256 n = vault.purchaseCount();
        if (n == 0) return;
        vm.prank(attestor);
        try vault.settleRune(id % n, bound(amount, 1, 1e12), keccak256(abi.encode(id)), 1) {} catch {}
    }

    function writeOff(uint256 id) external {
        uint256 n = vault.purchaseCount();
        if (n == 0) return;
        vm.prank(owner);
        try vault.writeOff(id % n, "w") {} catch {}
    }

    function giveBack(uint256 id, uint96 amount) external {
        uint256 n = vault.purchaseCount();
        if (n == 0) return;
        amount = uint96(bound(amount, 1, 1 ether));
        vm.deal(address(this), amount);
        try vault.returnUnspent{value: amount}(id % n) { ghostReturned += amount; } catch {}
    }

    function wait(uint32 s) external { Clock.skip(bound(s, 1, 2 days)); }

    /// the ordinary working path, so a campaign spends its calls on money that really moves
    function capThenDeclare(uint96 cap, uint96 maxEth, uint8 q) external {
        this.postCap(cap);
        this.declare(maxEth, q);
    }

    function waitAnHourThenRelease(uint256 id) external {
        Clock.skip(1 hours);
        this.release(id);
    }

    /// anyone with any key tries every way in that is not a declared purchase
    function tryToTake(uint8 who, uint8 how) external {
        address[5] memory actors = [owner, keeper, purchaser, attestor, address(0xBAD)];
        bytes[4] memory calls = [abi.encodeWithSignature("withdraw(uint256)", 1 ether), abi.encodeWithSignature("withdraw()"),
            abi.encodeWithSignature("sweep(address)", actors[who % 5]), abi.encodeWithSignature("release(uint256)", type(uint256).max)];
        uint256 before = address(vault).balance;
        vm.prank(actors[who % 5]);
        (bool ok,) = address(vault).call(calls[how % 4]);
        if (ok || address(vault).balance != before) violations++;
    }
}

contract AssetVaultInvariantTest is Base {
    VaultHandler handler;

    function setUp() public override {
        super.setUp();
        launcher.set(coinA, router, creatorA, 1);
        launcher.set(coinB, router, creatorB, 1);
        handler = new VaultHandler(runeVault, owner, keeper, purchaser, attestor, router, coinA, coinB, treasury);
        targetContract(address(handler));
    }

    /// ETH leaves only through released declarations, only to the purchaser, never more than was declared under a cap.
    function invariant_ethOutIsExactlyWhatWasReleased() public view {
        assertEq(handler.violations(), 0);
        assertEq(purchaser.balance, handler.ghostReleased());
        assertEq(address(runeVault).balance, handler.ghostDeposited() + handler.ghostReturned() + handler.ghostBonded() - handler.ghostReleased());
    }

    /// What is attributed to coins is exactly what the vault holds, and promises never exceed it.
    function invariant_theBooksMatchTheBalance() public view {
        // the vault holds exactly what it owes the coins plus the bond — a slash moves ETH from the second to the first, never out
        // (docs/phase3.md H1: fees of a coin still under MIN_ENTRY are `waiting`: owed to it, held here, not yet spendable)
        assertEq(runeVault.attributed() + runeVault.waiting() + runeVault.bond(), address(runeVault).balance);
        assertLe(runeVault.reserved(), runeVault.attributed());
        assertLe(runeVault.openPurchases().length, runeVault.MAX_OPEN());
    }

    /// Not a vacuous harness: driven by hand along the ordinary path, the same handler really does move money, and the same
    /// invariants hold afterwards. (A random campaign reaches a release only sometimes; this one always does.)
    function test_theHandlerReallyMovesMoney() public {
        handler.deposit(10 ether, false);
        handler.depositFromTreasury(5 ether, true);
        handler.capThenDeclare(1 ether, 3 ether, 3);
        handler.tryToTake(2, 3);
        handler.waitAnHourThenRelease(0);
        handler.settle(0, 1_000_000);
        handler.giveBack(0, 0.25 ether);
        handler.capThenDeclare(2 ether, 4 ether, 2);
        handler.cancel(1);
        assertEq(handler.ghostReleased(), 3 ether);
        assertEq(purchaser.balance, 3 ether);
        invariant_ethOutIsExactlyWhatWasReleased();
        invariant_theBooksMatchTheBalance();
    }
}
