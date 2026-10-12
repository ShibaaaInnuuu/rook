// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {FloorBase} from "../floor/FloorBase.sol";
import {MockSeeder} from "../utils/CurveBase.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";
import {SweepVault} from "../../src/SweepVault.sol";
import {FrogCurve} from "../../src/curve/FrogCurve.sol";
import {CurveFactory} from "../../src/curve/CurveFactory.sol";
import {CurveToken} from "../../src/curve/CurveToken.sol";
import {IItemQueue} from "../../src/interfaces/IItemQueue.sol";

/// B-07 (partial in the second review), docs/audit/2026-09-26/review2/fixes-R3-contracts.md §5: a public mint may not take the
/// ETH-backed room a Queued graduation is waiting for (the queued budgets count against `maxEthBacked`). The front-run of a
/// closing buy stays possible, bounded by the public cap; it costs the curve a delay (`QueueFull`), never a refund.
contract R3GradRoom is FloorBase {
    CurveFactory factory;
    MockSeeder seeder;
    SweepVault curveRaffle;
    SweepVault curveBurn;
    address creator = makeAddr("creator");
    address curveBuyer = makeAddr("curve buyer");
    address griefer = makeAddr("griefer");

    function _template() internal pure returns (FrogCurve.Config memory c) {
        c.supply = 1_000_000_000e18;
        c.reserved = 200_000_000e18;
        c.platformFeeCap = 0.25 ether;
        c.platformBps = 500;
        c.poolEthBps = 1_260;
        c.feeBps = 100;
        c.vaultShareBps = 5_000;
        c.creatorShareBps = 3_000;
        c.window = 3 days;
    }

    function _params() internal pure override returns (FloorModule.Params memory p) {
        p = super._params();
        p.maxEthBacked = 14e18;                  // room for two graduations and a little public minting
    }

    function setUp() public override {
        super.setUp();
        seeder = new MockSeeder(IERC20(address(wrapper)));
        CurveFactory.Targets memory t = CurveFactory.Targets(5 ether, 8 ether, 2 ether, 30 ether, 100, 800);
        factory = new CurveFactory(wrapper, seeder, IItemQueue(address(module)), platform, _template(), t, PBS);
        curveRaffle = new SweepVault(registry, RAFFLE_LISTING, wrapper, SweepVault.Policy.RAFFLE, address(venue), factory, desk, PBS);
        curveBurn = new SweepVault(registry, BURN_LISTING, wrapper, SweepVault.Policy.BURN, address(venue), factory, desk, PBS);
        _ceiling(curveRaffle);
        _ceiling(curveBurn);
        factory.wire(curveRaffle, curveBurn);
        module.wire(factory);
        _postCap(curveRaffle, 0.646 ether);
        seeder.setDepth(2 ether);
    }

    function _launch() internal returns (FrogCurve c, CurveToken tok) {
        vm.prank(creator);
        (address coin, address curve) = factory.launch("Alien", "ALIEN", SweepVault.Policy.RAFFLE, false);
        c = FrogCurve(payable(curve));
        tok = CurveToken(coin);
    }

    function _close(FrogCurve c) internal {
        uint256 amount = c.raise() * 2;
        vm.deal(curveBuyer, curveBuyer.balance + amount);
        vm.prank(curveBuyer);
        c.buy{value: amount}(0);
    }

    /// A graduation queued on a stale floor keeps its room: a public mint that would take it is refused, a smaller one is not,
    /// and the queued graduation fills at the next post.
    function test_B07_aPublicMintCannotTakeAQueuedGraduationsRoom() public {
        (FrogCurve c1,) = _launch();
        _close(c1);                                              // fills at once: ~6.32 units
        (FrogCurve c2, CurveToken tok2) = _launch();
        _skip(module.FLOOR_TTL());                               // stale: the second close queues
        _close(c2);
        assertEq(uint8(module.grad(address(tok2)).status), uint8(FloorModule.GradStatus.Queued));
        _post(F);
        uint256 need = module.gradEth() * 1e18 / module.gradPrice() + 1;
        uint256 room = 14e18 - wrapper.ethBacked() - need;       // what public mints may still add
        assertLt(room, 2e18);

        // the griefer tries to take the room with the public cap's worth (3 units): refused
        uint256 eth = 3 * module.mintPrice();
        vm.deal(griefer, eth);
        vm.prank(griefer);
        vm.expectRevert(FloorModule.OverBacked.selector);
        module.mint{value: eth}(griefer, 0);
        (uint256 unitPrice, uint256 maxIn) = module.mintCapacity();
        assertLe(maxIn * 1e18 / unitPrice, room, "the capacity shown leaves the queued room");

        // a mint inside the room goes through, and the graduation still fills
        _mint(griefer, maxIn);
        module.fill(address(tok2));
        assertEq(uint8(module.grad(address(tok2)).status), uint8(FloorModule.GradStatus.Filled));
        _books();
    }

    /// The accepted residual, measured: public minting can hold at most `publicCap / (1 − reserveBps)` of `maxEthBacked`, so a
    /// closing buy may meet `QueueFull` (the curve keeps trading), and closes once conversions or burns free the room.
    function test_B07_aFrontRunCostsAClosingBuyADelayNotARefund() public {
        (FrogCurve c1,) = _launch();
        _close(c1);                                              // ~6.32 of 14 used by the first graduation
        (FrogCurve c2,) = _launch();
        (uint256 unitPrice, uint256 maxIn) = module.mintCapacity();
        _mint(griefer, maxIn);                                   // the griefer takes what the public cap allows
        assertLe(wrapper.ethBacked() - 6.33e18, 10e18, "public units within the public cap");
        assertFalse(c2.canClose(), "the closing buy would meet QueueFull");
        uint256 amount = c2.raise() * 2;
        vm.deal(curveBuyer, amount);
        vm.prank(curveBuyer);
        vm.expectRevert(FrogCurve.QueueFull.selector);
        c2.buy{value: amount}(0);
        assertEq(uint8(c2.state()), uint8(FrogCurve.State.Trading), "no refund: the curve keeps trading");
        // the griefer's capital comes back through burns (at the spread); the room with it
        uint256 held = wrapper.balanceOf(griefer);
        for (uint256 i = 0; i < 4 && held > 0; ++i) {
            uint256 u = held > 5e18 ? 5e18 : held;
            _burn(griefer, u);
            held -= u;
            _skip(1 hours);
            _post(F);
        }
        assertTrue(c2.canClose(), "the room is back");
        unitPrice;
    }
}
