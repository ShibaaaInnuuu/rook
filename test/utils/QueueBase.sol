// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SweepBase} from "./SweepBase.sol";
import {ItemQueue} from "../../src/curve/ItemQueue.sol";
import {ICapSource} from "../../src/interfaces/ICapSource.sol";
import {ILiquiditySeeder} from "../../src/interfaces/ILiquiditySeeder.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {MockSeeder} from "./CurveBase.sol";
import {SweepVault} from "../../src/SweepVault.sol";

/// A curve as the queue sees it: queues a budget with a deadline, takes back its units and tail, or cancels. Test-only.
contract MockCurve {
    ItemQueue public immutable q;
    address public immutable coin;
    constructor(ItemQueue q_) { q = q_; coin = address(this); }
    receive() external payable { require(msg.sender == address(q)); }
    function enqueue(uint64 at, uint64 atBlock) external payable { q.enqueue{value: msg.value}(coin, at, atBlock); }
    function release() external returns (uint256, uint256, uint256) { return q.release(coin); }
    function openRefunds() external { q.cancel(coin); }
}

/// The collection's factory, as the queue reads it: who each coin's curve is, and the keeper's cap (a real vault's).
contract QueueCapSource is ICapSource {
    address public wrapped;
    SweepVault public vault;
    ILiquiditySeeder public seeder;
    mapping(address => address) public routerOf;
    constructor(address w, SweepVault v) { wrapped = w; vault = v; seeder = new MockSeeder(IERC20(address(0))); }
    function set(address coin, address curve) external { routerOf[coin] = curve; }
    function freshCap() external view returns (uint256, bool) { return (vault.cap(), vault.capFresh()); }
}

abstract contract QueueBase is SweepBase {
    ItemQueue queue;
    QueueCapSource src;
    uint256 constant CAP = 0.646 ether;

    function setUp() public virtual override {
        super.setUp();
        queue = new ItemQueue(wrapper, payee, 500, 0.001 ether, 1_000, PBS);
        src = new QueueCapSource(address(wrapper), raffle);
        queue.wire(src);
        _postCap(raffle, CAP);
    }

    function _coin(uint256 budget) internal returns (MockCurve c) {
        c = new MockCurve(queue);
        src.set(address(c), address(c));
        vm.deal(address(this), address(this).balance + budget);
        c.enqueue{value: budget}(uint64(block.timestamp + 3 days), uint64(block.number + 3 days / 12));
    }

    function _item(uint256 cost) internal {
        _deposit(address(queue));
        if (!raffle.capFresh()) _postCap(raffle, CAP);
        vm.prank(custodian);
        queue.recordItem(cost);
    }

    function _books() internal view {
        assertEq(address(queue).balance, queue.queued() + queue.heldDust(), "queue ETH == queued + held tails");
        assertEq(queue.totalIn(), queue.totalSpent() + queue.queued() + queue.heldDust() + queue.totalReturned()
            + queue.totalDustReleased(), "in == spent + queued + held + returned + released");
        assertGe(wrapper.balanceOf(address(queue)), queue.unitsHeld(), "the queue holds every unit it owes");
    }
}
