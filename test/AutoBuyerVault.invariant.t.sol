// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {AutoBase} from "./utils/AutoBase.sol";
import {AutoBuyerVault} from "../src/AutoBuyerVault.sol";
import {Clock} from "./utils/Clock.sol";

/// Every actor, with every key, in any order — and, unlike v1's campaigns, a handler that looks at WHY a call reverted.
/// @dev docs/security/internal-review-v1.md I5. v1's handlers wrapped every call in a bare `try … catch {}`, so a panic in a
/// transition that MUST be reachable was swallowed and recorded as "that call wasn't valid in this state"; the books then
/// balanced, because nothing had happened. That is exactly how C1 — a one-wei arithmetic underflow that bricked a vault for
/// ever — survived 200,000 handler calls. In a system whose central promise is "there is no withdraw function", liveness IS
/// the safety property, so here every catch inspects the revert data and an arithmetic panic anywhere is a violation.
contract AutoHandler is Test {
    AutoBuyerVault public vault;
    address owner; address keeper; address buyer; address router; address treasury; address[2] coins;

    uint256 public ghostDeposited;
    uint256 public ghostPulled;
    uint256 public ghostReturned;
    uint256 public ghostDealtToBuyer;       // ETH handed to the buyer for a refund, spent or not
    uint256 public ghostReturnedByBuyer;
    uint256 public violations;
    uint256 public panics;              // a required transition that reverted with Panic(uint256). Must stay zero.
    uint256 public pulls;               // proof the campaign actually reached the interesting states
    uint256 public settles;
    uint256 public writeOffs;

    constructor(AutoBuyerVault v, address o, address k, address b, address r, address t, address c0, address c1) {
        vault = v; owner = o; keeper = k; buyer = b; router = r; treasury = t; coins = [c0, c1];
    }

    /// Panic(uint256) — 0x4e487b71. An underflow, an overflow, an out-of-bounds index or a division by zero.
    function _isPanic(bytes memory r) private pure returns (bool) {
        return r.length >= 4 && r[0] == 0x4e && r[1] == 0x48 && r[2] == 0x7b && r[3] == 0x71;
    }

    function deposit(uint96 amount, bool which) external {
        amount = uint96(bound(amount, 1, 50 ether));
        vm.deal(router, amount);
        vm.prank(router);
        vault.deposit{value: amount}(coins[which ? 1 : 0]);
        ghostDeposited += amount;
    }

    function depositFromTreasury(uint96 amount, bool which) external {
        amount = uint96(bound(amount, 1, 50 ether));
        vm.deal(treasury, amount);
        vm.prank(treasury);
        vault.depositFrom{value: amount}(coins[which ? 1 : 0]);
        ghostDeposited += amount;
    }

    function postCap(uint96 cap) external {
        vm.prank(keeper);
        vault.postCap(bound(cap, 1, 5 ether));
    }

    function announce(uint96 maxEth, uint8 q) external {
        uint256 quantity = bound(q, 1, 5) * 100;          // whole runes at divisibility 2
        uint256 id = vault.purchaseCount();
        uint256 boundNow = vault.boundFor(quantity);
        vm.prank(buyer);
        try vault.announce(bound(maxEth, 1, 30 ether), quantity, "h") {
            if (vault.purchase(id).maxEth > boundNow) violations++;          // never over the cap it was announced under
            if (vault.inFlight() > vault.maxInFlight()) violations++;
        } catch (bytes memory r) { if (_isPanic(r)) panics++; }
    }

    function pull(uint256 id) external {
        uint256 n = vault.purchaseCount();
        if (n == 0) return;
        id = id % n;
        AutoBuyerVault.Purchase memory p = vault.purchase(id);
        uint256 before = buyer.balance;
        vm.prank(buyer);
        try vault.pull(id) {
            pulls++;
            ghostPulled += p.maxEth;
            if (buyer.balance - before != p.maxEth) violations++;            // to the buyer, exactly, and nowhere else
            if (vm.getBlockTimestamp() < p.readyAt) violations++;            // never before the public hour
        } catch (bytes memory r) { if (_isPanic(r)) panics++; }
    }

    function settle(uint256 id, uint64 amount) external {
        uint256 n = vault.purchaseCount();
        if (n == 0) return;
        vm.prank(buyer);
        try vault.settleRune(id % n, bound(amount, 1, 1e12), keccak256(abi.encode(id)), 1) { settles++; }
        catch (bytes memory r) { if (_isPanic(r)) panics++; }
    }

    /// The transition that MUST be reachable from every overdue state — the one C1 made unreachable.
    function writeOff(uint256 id, bool asOwner) external {
        uint256 n = vault.purchaseCount();
        if (n == 0) return;
        id = id % n;
        vm.prank(asOwner ? owner : address(0xBAD));
        try vault.writeOff(id, "w") { writeOffs++; }
        catch (bytes memory r) { if (_isPanic(r)) panics++; }
    }

    /// Returns in arbitrary instalments, from anybody — the exact shape that broke v1.
    function giveBack(uint256 id, uint96 amount, bool asStranger) external {
        uint256 n = vault.purchaseCount();
        if (n == 0) return;
        amount = uint96(bound(amount, 1, 5 ether));
        address from = asStranger ? address(0xBAD) : buyer;
        vm.deal(from, from.balance + amount);               // ADD, never overwrite: the buyer's balance is the thing under test
        if (!asStranger) ghostDealtToBuyer += amount;
        vm.prank(from);
        try vault.returnFunds{value: amount}(id % n) {
            ghostReturned += amount;
            if (!asStranger) ghostReturnedByBuyer += amount;
        } catch (bytes memory r) { if (_isPanic(r)) panics++; }
    }

    function cancel(uint256 id) external {
        uint256 n = vault.purchaseCount();
        if (n == 0) return;
        vm.prank(owner);
        try vault.returnFunds(id % n) {} catch (bytes memory r) { if (_isPanic(r)) panics++; }
    }

    function wait(uint32 s) external { Clock.skip(bound(s, 1, 2 days)); }

    /// the ordinary working path, so a campaign spends its calls on money that really moves
    function capAnnounceWaitPull(uint96 cap, uint96 maxEth, uint8 q) external {
        this.postCap(cap);
        this.announce(maxEth, q);
        Clock.skip(1 hours);
        this.pull(vault.purchaseCount() == 0 ? 0 : vault.purchaseCount() - 1);
    }

    /// anyone with any key tries every way out that is not a pull
    function tryToTake(uint8 who, uint8 how) external {
        address[5] memory actors = [owner, keeper, buyer, treasury, address(0xBAD)];
        bytes[5] memory calls = [
            abi.encodeWithSignature("withdraw(uint256)", 1 ether),
            abi.encodeWithSignature("withdraw()"),
            abi.encodeWithSignature("sweep(address)", actors[who % 5]),
            abi.encodeWithSignature("setBuyer(address)", actors[who % 5]),
            abi.encodeWithSignature("pull(uint256)", type(uint256).max)
        ];
        uint256 before = address(vault).balance;
        vm.prank(actors[who % 5]);
        (bool ok,) = address(vault).call(calls[how % 5]);
        if (ok || address(vault).balance != before) violations++;
    }
}

contract AutoBuyerVaultInvariantTest is AutoBase {
    AutoHandler handler;

    function setUp() public override {
        super.setUp();
        launcher.set(coinA, router, creatorA, 1);
        launcher.set(coinB, router, creatorB, 1);
        handler = new AutoHandler(autoRune, owner, keeper, autoBuyer, router, treasury, coinA, coinB);
        targetContract(address(handler));
    }

    function _announcedNotPulled() internal view returns (uint256 sum) {
        uint256[] memory open = autoRune.openPurchases();
        for (uint256 i = 0; i < open.length; ++i) {
            AutoBuyerVault.Purchase memory p = autoRune.purchase(open[i]);
            if (p.status == AutoBuyerVault.Status.Announced) sum += p.maxEth;
        }
    }

    /// ETH leaves only by a pull, only to the buyer, and the balance is exactly what came in minus what went out.
    function invariant_ethOutIsExactlyWhatWasPulled() public view {
        assertEq(handler.violations(), 0);
        assertEq(autoBuyer.balance, handler.ghostPulled() + handler.ghostDealtToBuyer() - handler.ghostReturnedByBuyer(),
            "the buyer holds exactly what it pulled, less what it sent back");
        assertEq(address(autoRune).balance, handler.ghostDeposited() + handler.ghostReturned() - handler.ghostPulled());
    }

    /// What the vault holds is what it owes the coins, plus whatever an announcement has already spoken for.
    function invariant_theBooksMatchTheBalance() public view {
        assertEq(autoRune.attributed() + autoRune.waiting() + _announcedNotPulled(), address(autoRune).balance);
        assertLe(autoRune.inFlight(), autoRune.maxInFlight());
        assertLe(autoRune.openPurchases().length, autoRune.MAX_OPEN());
    }

    /// Every wei ever contributed is in exactly one of three places: unspent and attributed, waiting under MIN_ENTRY, or
    /// charged to a purchase that has not given it back. This is the ledger C1 broke.
    function invariant_everyContributedWeiIsAccountedFor() public view {
        assertEq(
            autoRune.attributed() + autoRune.waiting() + autoRune.chargedTo(coinA) + autoRune.chargedTo(coinB),
            autoRune.contributedBy(coinA) + autoRune.contributedBy(coinB)
        );
    }

    /// The liveness property v1 never asserted: no transition the design REQUIRES to be reachable ever reverts with a panic.
    function invariant_noRequiredTransitionEverPanics() public view {
        assertEq(handler.panics(), 0, "a required transition reverted with an arithmetic panic");
    }

    /// Not a vacuous harness: driven by hand along the ordinary path it really moves money, and every invariant still holds.
    function test_theHandlerReallyMovesMoney() public {
        handler.deposit(10 ether, false);
        handler.depositFromTreasury(5 ether, true);
        handler.capAnnounceWaitPull(1 ether, 3 ether, 3);
        handler.tryToTake(2, 3);
        assertEq(handler.pulls(), 1);
        handler.giveBack(0, 0.25 ether, true);
        handler.settle(0, 1_000_000);
        assertEq(handler.settles(), 1);
        handler.capAnnounceWaitPull(2 ether, 4 ether, 2);
        handler.wait(2 days);
        handler.wait(2 days);
        handler.writeOff(1, true);
        assertEq(handler.writeOffs(), 1, "an overdue purchase really can be written off");
        invariant_ethOutIsExactlyWhatWasPulled();
        invariant_theBooksMatchTheBalance();
        invariant_everyContributedWeiIsAccountedFor();
        invariant_noRequiredTransitionEverPanics();
    }
}
