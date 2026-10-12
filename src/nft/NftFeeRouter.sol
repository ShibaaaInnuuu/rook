// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {IPonsV2FeeEscrow, IPonsV2BondingCurve, IPonsV2MemeHook} from "../interfaces/IPons.sol";
import {INftRegistry, INftVault} from "./INft.sol";

/// @title NftFeeRouter
/// @notice One per coin, cloned by the NftLauncher (EIP-1167) and named to Pons as the coin's creator-fee recipient. It claims what
/// Pons has escrowed for it and sends 80% to the coin's NFT vault, attributed to the coin, and 20% to the treasury. The split is a
/// constant in the implementation's bytecode; there is no owner and no upgrade. docs/v4/nft.md §3.
/// @dev The one difference from `FeeRouter`: attribution follows the ESCROW'S RECORD, measured as the ETH `escrow.claim()` actually
/// delivered, not this contract's balance. ETH that reaches the router any other way — a direct payment — is not the coin's
/// fees, so it is not attributed: it goes to the treasury whole. (A payment through `escrow.credit(router)` IS in Pons's record and
/// is attributed; it is also real money bought for this coin's holders, by whoever chose to pay it.)
contract NftFeeRouter {
    using SafeERC20 for IERC20;

    uint256 public constant VAULT_BPS = 8_000;
    uint256 private constant BPS = 10_000;
    address public immutable launcher;

    address public coin;
    INftRegistry public registry;
    INftVault public vault;
    IPonsV2FeeEscrow public escrow;
    IPonsV2BondingCurve public curve;
    IPonsV2MemeHook public hook;
    uint256 public treasuryOwed;
    uint256 public earned;          // everything the escrow has paid this router: the coin's fees, by Pons's own record

    event Harvested(uint256 claimed, uint256 toVault, uint256 toTreasury);
    event TreasuryDeferred(uint256 owed);
    event TokenForwarded(address indexed token, address indexed treasury, uint256 amount);

    error AlreadyInitialised();
    error NotInitialised();
    error NotLauncher();
    error Reentered();

    uint256 private _entered;

    constructor() {
        launcher = msg.sender;
        coin = address(1);
    }

    function initialize(address coin_, INftRegistry registry_, INftVault vault_, IPonsV2FeeEscrow escrow_,
        IPonsV2BondingCurve curve_, IPonsV2MemeHook hook_) external {
        if (msg.sender != launcher) revert NotLauncher();
        if (coin != address(0)) revert AlreadyInitialised();
        if (coin_ == address(0)) revert NotInitialised();
        coin = coin_;
        registry = registry_;
        vault = vault_;
        escrow = escrow_;
        curve = curve_;
        hook = hook_;
    }

    receive() external payable {}

    function sweepPool(bytes32 poolId) external {
        hook.sweepPoolFees(poolId, 0, 0);
    }

    function forwardToken(IERC20 token) external returns (uint256 amount) {
        if (coin == address(0)) revert NotInitialised();
        amount = token.balanceOf(address(this));
        if (amount == 0) return 0;
        address to = registry.treasury();
        token.safeTransfer(to, amount);
        emit TokenForwarded(address(token), to, amount);
    }

    /// @notice Permissionless. Sweeps the curve, claims the escrow, splits what the escrow paid 80/20, and sends everything else
    /// this router holds to the treasury.
    function harvest() external returns (uint256 toVault, uint256 toTreasury) {
        if (coin == address(0)) revert NotInitialised();
        if (_entered == 1) revert Reentered();
        _entered = 1;
        try curve.sweepFees(0) {} catch {}
        uint256 claimed;
        if (escrow.balanceOf(address(this)) != 0) {
            uint256 before = address(this).balance;
            escrow.claim();
            claimed = address(this).balance - before;       // what arrived, not what the call said
            earned += claimed;
        }
        toVault = (claimed * VAULT_BPS) / BPS;
        toTreasury = address(this).balance - toVault;      // the treasury's 20%, anything owed, and any direct payment
        if (toVault != 0) vault.deposit{value: toVault}(coin);
        if (toTreasury != 0) {
            treasuryOwed = 0;
            (bool ok,) = registry.treasury().call{value: toTreasury}("");
            if (!ok) {
                treasuryOwed = toTreasury;
                emit TreasuryDeferred(toTreasury);
                toTreasury = 0;
            }
        }
        _entered = 0;
        emit Harvested(claimed, toVault, toTreasury);
    }
}
