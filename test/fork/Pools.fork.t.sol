// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IPoolManager} from "@uniswap/v4-core/src/interfaces/IPoolManager.sol";
import {PoolKey} from "@uniswap/v4-core/src/types/PoolKey.sol";
import {StateLibrary} from "@uniswap/v4-core/src/libraries/StateLibrary.sol";
import {TickMath} from "@uniswap/v4-core/src/libraries/TickMath.sol";
import {PoolBase} from "../utils/PoolBase.sol";
import {SweepVault} from "../../src/SweepVault.sol";
import {FrogCurve} from "../../src/curve/FrogCurve.sol";
import {CurveToken} from "../../src/curve/CurveToken.sol";

interface IHasPoolManager { function poolManager() external view returns (address); }
interface IPermit2Lite { function DOMAIN_SEPARATOR() external view returns (bytes32); }
interface IPositionManagerLite { function permit2() external view returns (address); function name() external view returns (string memory); }
interface IFeeController {
    function POOL_MANAGER() external view returns (address);
    function getFee(PoolKey memory key) external view returns (uint24);
    function triggerFeeUpdate(PoolKey memory key) external;
}

/// docs/v3/pools.md §1 and §3, on a LOCAL anvil fork of Robinhood Chain (4663) — nothing is sent to any public chain:
///   anvil --fork-url https://rpc.mainnet.chain.robinhood.com --port 8621
///   forge test --match-path "test/fork/Pools*" --fork-url http://127.0.0.1:8621
/// The measured v4 contracts answer as recorded; a curve graduates into the REAL PoolManager (with its protocol fee
/// controller, which the etched copy in the unit tests does not have); the router trades; the fees reach the vaults.
contract PoolsForkTest is PoolBase {
    using StateLibrary for IPoolManager;

    address constant POSITION_MANAGER = 0x58daec3116aae6D93017bAAea7749052E8a04fA7;
    address constant STATE_VIEW = 0xF3334192D15450CdD385c8B70e03f9A6bD9E673b;
    address constant UNIVERSAL_ROUTER = 0x8876789976dEcBfCbBbe364623C63652db8C0904;
    address constant UNIVERSAL_ROUTER_212 = 0x204FAca1764B154221e35c0d20aBb3c525710498;
    address constant QUOTER = 0x8Dc178eFB8111BB0973Dd9d722ebeFF267c98F94;
    address constant PERMIT2 = 0x000000000022D473030F116dDEE9F6B43aC78BA3;
    IFeeController constant FEE_CONTROLLER = IFeeController(0x6d0009504D129CF5002Dba61D9Ae8575AA79314c);
    bool forked;

    function setUp() public override {
        forked = block.chainid == 4663;
        if (!forked) return;
        super.setUp();
    }

    function test_theMeasuredContractsAnswerOnTheFork() public view {
        if (!forked) return;
        assertGt(address(PM).code.length, 20_000, "the PoolManager has code (the real one, not an etched copy)");
        assertEq(IHasPoolManager(POSITION_MANAGER).poolManager(), address(PM));
        assertEq(IPositionManagerLite(POSITION_MANAGER).permit2(), PERMIT2);
        assertEq(keccak256(bytes(IPositionManagerLite(POSITION_MANAGER).name())), keccak256("Uniswap v4 Positions NFT"));
        assertEq(IHasPoolManager(STATE_VIEW).poolManager(), address(PM));
        assertEq(IHasPoolManager(UNIVERSAL_ROUTER).poolManager(), address(PM));
        assertEq(IHasPoolManager(UNIVERSAL_ROUTER_212).poolManager(), address(PM));
        assertEq(IHasPoolManager(QUOTER).poolManager(), address(PM));
        assertEq(FEE_CONTROLLER.POOL_MANAGER(), address(PM));
        assertEq(IPermit2Lite(PERMIT2).DOMAIN_SEPARATOR(), keccak256(abi.encode(
            keccak256("EIP712Domain(string name,uint256 chainId,address verifyingContract)"), keccak256("Permit2"), 4663, PERMIT2)));
        assertEq(address(PM).codehash, keccak256(vm.parseBytes(vm.readFile("test/fixtures/v4-pool-manager-4663.hex"))),
            "the unit tests' etched copy is exactly this code");
    }

    function test_aCurveGraduatesIntoTheRealPoolManager() public {
        if (!forked) return;
        (FrogCurve c, CurveToken tok) = _graduate(SweepVault.Policy.RAFFLE);
        PoolKey memory ck = lock.coinKey(address(tok));
        PoolKey memory sk = lock.sharedKey();
        (,, uint128 liq) = lock.coins(address(tok));
        (int24 lo, int24 hi) = lock.coinRange(address(tok));
        assertEq(_pmLiquidity(ck, lo, hi), liq, "the coin's pool, in the real manager");
        assertEq(lock.positionCount(), 4, "near, wide, far, bid in the shared pool");
        for (uint256 i = 0; i < 4; ++i) {
            bytes32 id = lock.positionIds(i);
            assertEq(_pmLiquidity(sk, lock.position(id).lower, lock.position(id).upper), lock.position(id).liquidity);
        }
        emit log_named_uint("coin pool liquidity", liq);
        emit log_named_uint("shared positions", lock.positionCount());

        // Uniswap's protocol fee: what the controller says for our keys, applied by its permissionless trigger
        uint24 coinFee = FEE_CONTROLLER.getFee(ck);
        uint24 sharedFee = FEE_CONTROLLER.getFee(sk);
        emit log_named_uint("controller fee, coin pool (1% LP)", coinFee);
        emit log_named_uint("controller fee, shared pool (0.3% LP)", sharedFee);
        FEE_CONTROLLER.triggerFeeUpdate(ck);
        FEE_CONTROLLER.triggerFeeUpdate(sk);
        (,, uint24 pfCoin,) = PM.getSlot0(ck.toId());
        (,, uint24 pfShared,) = PM.getSlot0(sk.toId());
        assertEq(pfCoin, coinFee);
        assertEq(pfShared, sharedFee);

        // trade through our router, then collect: the fees reach the coin's vault
        address t = makeAddr("fork trader");
        vm.deal(t, 5 ether);
        vm.prank(t);
        uint256 got = etchRouter.buy{value: 1 ether}(address(tok), 1, t, block.timestamp);
        vm.startPrank(t);
        tok.approve(address(etchRouter), got);
        uint256 back = etchRouter.sell(address(tok), got, 1, t, block.timestamp);
        vm.stopPrank();
        emit log_named_uint("1 ETH in, round trip out (wei)", back);
        assertGt(back, 0.9 ether);
        uint256 unitsBefore = curveRaffle.units(address(tok));
        (uint256 tokenFees, uint256 quoteFees) = lock.collectCoinFees(address(tok));
        assertGt(tokenFees, 0);
        assertGt(quoteFees, 0);
        assertEq(curveRaffle.units(address(tok)) - unitsBefore, quoteFees / 2);
        (uint256 e, uint256 w) = lock.claimShared(address(tok));
        assertGt(e + w, 0, "seeder fees from the shared pool");
        assertEq(IERC20(address(wrapper)).balanceOf(address(etchRouter)), 0);
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduated));
    }
}
