// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SweepBase} from "./SweepBase.sol";
import {SweepVault} from "../../src/SweepVault.sol";
import {FrogCurve} from "../../src/curve/FrogCurve.sol";
import {CurveFactory} from "../../src/curve/CurveFactory.sol";
import {CurveToken} from "../../src/curve/CurveToken.sol";
import {ItemQueue} from "../../src/curve/ItemQueue.sol";
import {ILiquiditySeeder} from "../../src/interfaces/ILiquiditySeeder.sol";

/// Stands in for the LiquidityLock where a test is about the curve, not the pools: takes exactly what it is given and keeps it
/// (no way out, like the lock). Its depth (`sharedEthIn`) can be set, so a test can put a collection past its early stage.
contract MockSeeder is ILiquiditySeeder {
    IERC20 public immutable wrapped;
    uint256 public tokenAmount; uint256 public wrappedForToken; uint256 public wrappedForShared; uint256 public ethIn;
    address public token;
    bool public short;
    bool public viaEth;
    uint256 public sharedEthIn;
    constructor(IERC20 wrapped_) { wrapped = wrapped_; }
    function setShort(bool s) external { short = s; }
    function setDepth(uint256 d) external { sharedEthIn = d; }
    function seed(address token_, uint256 tokenAmount_, uint256 wft, uint256 wfs) external payable {
        token = token_; tokenAmount = tokenAmount_; wrappedForToken = wft; wrappedForShared = wfs; ethIn = msg.value;
        sharedEthIn += msg.value;
        IERC20(token_).transferFrom(msg.sender, address(this), short ? tokenAmount_ - 1 : tokenAmount_);
        wrapped.transferFrom(msg.sender, address(this), wft + wfs);
    }
    function seedEth(address token_, uint256 tokenAmount_) external payable {
        token = token_; tokenAmount = tokenAmount_; ethIn = msg.value; viaEth = true;
        IERC20(token_).transferFrom(msg.sender, address(this), tokenAmount_);
    }
    uint256 public protocolEth;
    function addProtocolLiquidity() external payable { protocolEth += msg.value; }
}

/// `SweepBase`, plus our own launch curve for the frog collection: a factory with the worked example's terms (docs/v3/curve.md)
/// — 1B supply, 200M reserved, the raise split 5% platform (capped at 0.25 ETH) / 12.6% shared-pool ETH side / the rest to the
/// collection's buy queue, a 1% fee split 50/30/20, a three-day window, targets 5 ETH normal and 8 ETH early — its own queue
/// (5% buffer over the keeper's cap, 0.001 ETH dust), and its own pair of policy vaults whose launcher is the factory. The
/// keeper posts a cap of 0.646 ETH per frog (a 0.02 BTC floor at ETH/BTC 0.0316, plus 2% conversion).
abstract contract CurveBase is SweepBase {
    CurveFactory factory;
    ItemQueue queue;
    MockSeeder seeder;
    SweepVault curveRaffle;
    SweepVault curveBurn;
    address platform = makeAddr("platform");
    address creator = makeAddr("creator");

    uint256 constant SUPPLY = 1_000_000_000e18;
    uint256 constant RESERVED = 200_000_000e18;
    uint256 constant NORMAL = 5 ether;
    uint256 constant EARLY = 8 ether;
    uint256 constant CAP = 0.646 ether;

    function _template() internal pure virtual returns (FrogCurve.Config memory c) {
        c.supply = SUPPLY;
        c.reserved = RESERVED;
        c.platformFeeCap = 0.25 ether;
        c.platformBps = 500;
        c.poolEthBps = 1_260;
        c.feeBps = 100;
        c.vaultShareBps = 5_000;
        c.creatorShareBps = 3_000;
        c.window = 3 days;
    }

    function _targets() internal pure virtual returns (CurveFactory.Targets memory t) {
        t.normalRaise = NORMAL;
        t.earlyRaise = EARLY;
        t.earlyUntilEth = 2 ether;
        t.maxRaise = 30 ether;
    }

    function setUp() public virtual override {
        super.setUp();
        seeder = new MockSeeder(IERC20(address(wrapper)));
        queue = new ItemQueue(wrapper, payee, 500, 0.001 ether, 1_000, PBS);
        factory = new CurveFactory(wrapper, seeder, queue, platform, _template(), _targets(), PBS);
        curveRaffle = new SweepVault(registry, RAFFLE_LISTING, wrapper, SweepVault.Policy.RAFFLE, address(venue), factory, desk, PBS);
        curveBurn = new SweepVault(registry, BURN_LISTING, wrapper, SweepVault.Policy.BURN, address(venue), factory, desk, PBS);
        _ceiling(curveRaffle);
        _ceiling(curveBurn);
        factory.wire(curveRaffle, curveBurn);
        queue.wire(factory);
        _postCap(curveRaffle, CAP);
        seeder.setDepth(2 ether);            // most tests: a collection past its early stage, so targets are the normal 5 ETH
    }

    function _launch(SweepVault.Policy policy) internal returns (FrogCurve c, CurveToken tok) {
        return _launchAs(policy, false);
    }

    function _launchAs(SweepVault.Policy policy, bool sweepOnly) internal returns (FrogCurve c, CurveToken tok) {
        vm.prank(creator);
        (address coin, address curve) = factory.launch("Alien", "ALIEN", policy, sweepOnly);
        c = FrogCurve(payable(curve));
        tok = CurveToken(coin);
    }

    function _buy(FrogCurve c, address who, uint256 eth) internal returns (uint256 out) {
        vm.deal(who, who.balance + eth);
        vm.prank(who);
        out = c.buy{value: eth}(0);
    }

    /// Buy the whole curve (the raise plus the fee, with a little over to check the change comes back).
    function _fill(FrogCurve c, address who) internal {
        _buy(c, who, c.raise() * 2);
    }

    /// The canister buys one item for the queue: wraps it there (a deposit from custody), then records its cost.
    function _item(uint256 cost) internal {
        _deposit(address(queue));
        if (!curveRaffle.capFresh()) _postCap(curveRaffle, CAP);
        vm.prank(custodian);
        queue.recordItem(cost);
    }

    /// Buy items at `cost` until `c`'s coin is filled.
    function _fillQueue(FrogCurve c, uint256 cost) internal returns (uint256 items) {
        while (!queue.isFilled(address(c.token()))) { _item(cost); ++items; }
    }

    function _curveBooks(FrogCurve c) internal view {
        assertEq(address(c).balance, c.reserveEth() + c.feesForVault() + c.creatorOwed() + c.platformOwed(), "ETH == raise + fees owed");
    }

    function _queueBooks() internal view {
        assertEq(address(queue).balance, queue.queued() + queue.heldDust(), "queue ETH == queued + dust held");
        assertEq(queue.totalIn(), queue.totalSpent() + queue.queued() + queue.heldDust() + queue.totalReturned()
            + queue.totalDustReleased(), "queue: in == spent + queued + dust + returned + released dust");
        assertGe(wrapper.balanceOf(address(queue)), queue.unitsHeld(), "queue holds the units it owes");
    }
}
