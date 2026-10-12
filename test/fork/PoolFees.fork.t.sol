// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test, console2} from "forge-std/Test.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IPonsV2LaunchFactory, IPonsV2FeeEscrow} from "../../src/interfaces/IPons.sol";
import {ListingRegistry} from "../../src/ListingRegistry.sol";
import {Launcher} from "../../src/Launcher.sol";
import {FeeRouter} from "../../src/FeeRouter.sol";
import {AssetVault} from "../../src/AssetVault.sol";

struct PoolKey { address currency0; address currency1; uint24 fee; int24 tickSpacing; address hooks; }
struct SwapParams { bool zeroForOne; int256 amountSpecified; uint160 sqrtPriceLimitX96; }

interface IPoolManagerLite {
    function unlock(bytes calldata data) external returns (bytes memory);
    function swap(PoolKey memory key, SwapParams memory params, bytes calldata hookData) external returns (int256 delta);
    function settle() external payable returns (uint256);
    function sync(address currency) external;
    function take(address currency, address to, uint256 amount) external;
}

interface IHookRead {
    function feeSweepOperator() external view returns (address);
    function pendingFees(bytes32 poolId, address currency) external view returns (uint256);
    function pendingCreatorTax(bytes32 poolId, address currency) external view returns (uint256);
    function sweepPoolFees(bytes32 poolId, uint256 minConversionQuoteOut, uint256 minBuybackTokensOut) external;
}

interface IFactoryRead2 {
    function getLaunchedToken(address token) external view returns (
        address token_, address curve, address deployer, address creatorFeeRecipient, address pairToken, uint256 graduationThreshold,
        uint24 poolFee, int24 tickSpacing, uint16 creatorTaxBps, bool buybackEnabled, uint8 phase, uint256 sweptQuote, uint256 sweptTokens,
        uint256 sweptAt, bool exists);
    function createGraduatedPool(address token) external returns (uint256 positionId);
}

interface ICurveBuy {
    function buy(uint256 quoteIn, uint256 minTokensOut, address recipient) external payable returns (uint256);
    function graduated() external view returns (bool);
}

/// A trader that swaps straight against the v4 pool manager, exact-input, as a router would. Test-only.
contract PoolTrader {
    IPoolManagerLite immutable pm;
    constructor(IPoolManagerLite pm_) { pm = pm_; }
    receive() external payable {}

    function buy(PoolKey memory key, uint256 ethIn) external payable returns (uint256 tokensOut) {
        return abi.decode(pm.unlock(abi.encode(key, true, ethIn)), (uint256));
    }

    function sell(PoolKey memory key, uint256 tokensIn) external returns (uint256 ethOut) {
        return abi.decode(pm.unlock(abi.encode(key, false, tokensIn)), (uint256));
    }

    function unlockCallback(bytes calldata data) external returns (bytes memory) {
        require(msg.sender == address(pm));
        (PoolKey memory key, bool isBuy, uint256 amountIn) = abi.decode(data, (PoolKey, bool, uint256));
        // native ETH is currency0 (address zero sorts first): a buy is zeroForOne
        uint160 limit = isBuy ? 4295128740 : 1461446703485210103287273052203988822378723970341;
        int256 delta = pm.swap(key, SwapParams(isBuy, -int256(amountIn), limit), "");
        int128 a0 = int128(delta >> 128);
        int128 a1 = int128(delta);
        if (isBuy) {
            pm.settle{value: uint256(uint128(-a0))}();
            pm.take(key.currency1, address(this), uint256(uint128(a1)));
            return abi.encode(uint256(uint128(a1)));
        }
        pm.sync(key.currency1);
        IERC20(key.currency1).transfer(address(pm), uint256(uint128(-a1)));
        pm.settle();
        pm.take(address(0), address(this), uint256(uint128(a0)));
        return abi.encode(uint256(uint128(a0)));
    }
}

/// docs/phase1.md §1: what reaches the router from POOL trades after graduation. Fork of 4663, public RPC; nothing is sent.
/// The sweep operator is impersonated — a fork can pretend to be anyone — to see what its sweep does to our router.
contract PoolFeesForkTest is Test {
    IPonsV2LaunchFactory constant FACTORY = IPonsV2LaunchFactory(0x7eD598BcEf8bd9Edd8C97A195C6d13f40801EC7e);
    IPonsV2FeeEscrow constant ESCROW = IPonsV2FeeEscrow(0xd3AFEB2a57f70eF218Aa82451c51B2fb0416Ac9e);
    IPoolManagerLite constant PM = IPoolManagerLite(0x8366a39CC670B4001A1121B8F6A443A643e40951);

    ListingRegistry registry;
    Launcher launcher;
    PoolTrader trader;
    IHookRead hook;
    address owner = makeAddr("owner");
    address treasury = makeAddr("treasury");
    address creator = makeAddr("creator");
    address whale = makeAddr("whale");
    bool forked;

    function setUp() public {
        forked = block.chainid == 4663;
        if (!forked) return;
        hook = IHookRead(FACTORY.memeHook());
        vm.startPrank(owner);
        registry = new ListingRegistry(owner, treasury);
        launcher = new Launcher(FACTORY, registry);
        registry.wire(address(launcher), address(0xD157));
        registry.listRune("TEST RUNE", "840000:1", 2, "bc1p-test-vault");
        vm.stopPrank();
        trader = new PoolTrader(PM);
        vm.deal(creator, 1 ether);
        vm.deal(whale, 200 ether);
        vm.deal(address(trader), 200 ether);
    }

    function _launchAndGraduate(uint16 taxBps, bytes32 salt) internal returns (address coin, FeeRouter router, PoolKey memory key, bytes32 poolId) {
        Launcher.LaunchParams memory p;
        p.name = "Pool Test"; p.symbol = "POOL"; p.salt = salt; p.creatorTaxBps = taxBps;
        uint256 fee = FACTORY.launchFee();
        vm.prank(creator);
        (address coin_, address curve, address router_) = launcher.launch{value: fee}(p);
        coin = coin_; router = FeeRouter(payable(router_));
        vm.warp(vm.getBlockTimestamp() + 60);
        for (uint256 i = 0; i < 12 && !ICurveBuy(curve).graduated(); ++i) {
            vm.prank(whale);
            try ICurveBuy(curve).buy{value: 1 ether}(1 ether, 0, whale) {} catch { break; }
        }
        assertTrue(ICurveBuy(curve).graduated(), "graduated on the fork");
        // graduation is two steps in Pons V2: the curve is drained at the threshold, and seeding the v4 pool is a second,
        // permissionless call (`createGraduatedPool`) — on the live chain Pons's own keeper makes it within moments
        (,,,,,,,,,, uint8 phase,,,,) = IFactoryRead2(address(FACTORY)).getLaunchedToken(coin);
        if (phase == 1) IFactoryRead2(address(FACTORY)).createGraduatedPool(coin);
        router.harvest();                                   // clear what the curve and graduation left, so the pool is measured alone
        (,,,,,, uint24 poolFee, int24 tickSpacing,,,,,,,) = IFactoryRead2(address(FACTORY)).getLaunchedToken(coin);
        key = PoolKey(address(0), coin, poolFee, tickSpacing, address(hook));
        poolId = keccak256(abi.encode(key));
    }

    function test_whoMaySweepWhat() public {
        if (!forked) return;
        (address coin, FeeRouter router, PoolKey memory key, bytes32 poolId) = _launchAndGraduate(200, keccak256("etch-pool-1"));

        // an ordinary exact-input BUY pays its fee in the COIN (the unspecified side)
        uint256 got = trader.buy(key, 1 ether);
        assertGt(got, 0);
        assertGt(hook.pendingFees(poolId, coin), 0, "a buy's hook fee is coin-denominated");
        assertGt(hook.pendingCreatorTax(poolId, coin), 0, "and so is its creator tax");
        assertEq(hook.pendingFees(poolId, address(0)), 0);

        // with ANY coin-denominated fee pending, our router may not sweep at all — not even an ETH half
        IERC20(coin).approve(address(PM), type(uint256).max);
        trader.sell(key, got / 2);
        assertGt(hook.pendingFees(poolId, address(0)), 0, "a sell's fee is in ETH");
        vm.expectRevert();                                  // InternalSwapRequiresOperator
        router.sweepPool(poolId);
        assertEq(ESCROW.balanceOf(address(router)), 0, "so nothing reaches our router until Pons's operator sweeps");

        // a stranger may not sweep either
        vm.prank(whale);
        vm.expectRevert();
        hook.sweepPoolFees(poolId, 1, 0);

        // Pons's operator sweeps (impersonated): the coin half is converted inside the pool, and ETH is credited to OUR router
        vm.prank(hook.feeSweepOperator());
        hook.sweepPoolFees(poolId, 1, 0);
        assertGt(ESCROW.balanceOf(address(router)), 0);
        assertEq(hook.pendingFees(poolId, coin), 0);
        assertEq(IERC20(coin).balanceOf(address(router)), 0, "the router is never handed the coin by a sweep");

        // after the operator's sweep, with only sells since, the ETH-only state is one the router CAN sweep itself
        trader.sell(key, got / 4);
        assertEq(hook.pendingFees(poolId, coin), 0);
        uint256 before = ESCROW.balanceOf(address(router));
        router.sweepPool(poolId);
        assertGt(ESCROW.balanceOf(address(router)), before, "ETH-only fees: the router's own sweep works");
    }

    function _takePerEth(uint16 taxBps, bytes32 salt) internal returns (uint256 takeWeiPerEth, uint256 toVault, uint256 volume) {
        (address coin, FeeRouter router, PoolKey memory key, bytes32 poolId) = _launchAndGraduate(taxBps, salt);
        IERC20(coin).approve(address(PM), type(uint256).max);
        vm.prank(address(trader));
        IERC20(coin).approve(address(PM), type(uint256).max);
        // ten small round trips (0.05 ETH against a ~4 ETH pool), so the measurement is of fees and not of our own price impact.
        // Volume is the ETH that crossed the pool in both directions.
        for (uint256 i = 0; i < 10; ++i) {
            uint256 got = trader.buy(key, 0.05 ether);
            uint256 back = trader.sell(key, got);
            volume += 0.05 ether + back;
        }
        // the operator's conversion is capped at a 3% price move per sweep; what does not fit waits for the next one
        for (uint256 i = 0; i < 4 && (hook.pendingFees(poolId, coin) != 0 || hook.pendingFees(poolId, address(0)) != 0 || i == 0); ++i) {
            vm.prank(hook.feeSweepOperator());
            hook.sweepPoolFees(poolId, 1, 0);
        }
        assertEq(hook.pendingFees(poolId, coin), 0, "fully converted");
        (uint256 v, uint256 t) = router.harvest();
        toVault = v;
        takeWeiPerEth = ((v + t) * 1 ether) / volume;
    }

    function test_takePerEthOfPoolVolume_at0_2_10pct() public {
        if (!forked) return;
        uint16[3] memory taxes = [uint16(0), 200, 1000];
        for (uint256 i = 0; i < 3; ++i) {
            (uint256 take, uint256 toVault, uint256 volume) = _takePerEth(taxes[i], keccak256(abi.encode("etch-take", i)));
            console2.log("creator tax (bps):", taxes[i]);
            console2.log("  router take per 1 ETH of pool volume (wei):", take);
            console2.log("  of which the vault's 80% (wei per ETH):", (take * 8000) / 10000);
            console2.log("  volume measured (wei):", volume, " vault received (wei):", toVault);
            // theory: 70% of the 1% hook fee + the whole creator tax
            uint256 theory = 0.007 ether + (uint256(taxes[i]) * 1 ether) / 10000;
            // at a high tax the figure runs a little ABOVE theory, because volume here is what the trader paid in and got
            // back, and what a seller gets back is already net of the tax
            assertApproxEqRel(take, theory, 0.06e18, "within 6% of 0.7% + tax");
        }
    }
}
