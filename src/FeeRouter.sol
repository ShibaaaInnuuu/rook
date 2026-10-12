// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IPonsV2FeeEscrow, IPonsV2BondingCurve, IPonsV2MemeHook} from "./interfaces/IPons.sol";
import {ListingRegistry} from "./ListingRegistry.sol";
import {IEtchVault} from "./interfaces/IEtchVault.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";

/// @title FeeRouter
/// @notice One per coin, cloned by the Launcher and named to Pons as the coin's creator-fee recipient. It can do one thing: claim
/// what Pons has escrowed for it and send 80% to the coin's vault, attributed to the coin, and 20% to the treasury.
/// It has no owner, no upgrade path, and no function that calls Pons's `transferCreatorFeeRecipient`, so it cannot hand its role on.
/// @dev Fees do not arrive in escrow by themselves: on the curve they must be swept, which this router may do itself; after
/// graduation the token-side half of the pool's fees waits for Pons's sweep operator. See docs/phase0.md §1.
/// What this does NOT protect against: Pons's protocol owner redirecting this coin's creator fees elsewhere (a 3-day public
/// timelock in Pons V2; docs/phase0.md §1). If that happens fees simply stop arriving here. Nor does it care where ETH came from:
/// anything sent to it or credited to it in escrow is split the same way.
contract FeeRouter {
    using SafeERC20 for IERC20;

    uint256 public constant VAULT_BPS = 8_000;   // the split: a constant in the implementation's bytecode
    uint256 private constant BPS = 10_000;
    /// @notice docs/phase3.md L1. Whoever deployed the implementation — the Launcher — and nobody else may initialise a clone.
    /// An immutable lives in the implementation's code, which every clone runs, so clones share it.
    address public immutable launcher;

    address public coin;
    ListingRegistry public registry;
    IEtchVault public vault;
    IPonsV2FeeEscrow public escrow;
    IPonsV2BondingCurve public curve;
    IPonsV2MemeHook public hook;
    uint256 public treasuryOwed;                 // the treasury's share when a payment to it failed; retried on every harvest

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
        coin = address(1);          // the implementation itself can never be initialised or harvested into anything
    }

    function initialize(address coin_, ListingRegistry registry_, IEtchVault vault_, IPonsV2FeeEscrow escrow_,
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

    /// @notice After graduation: sweep the pool's hook for this coin. Anyone may call it with the pool's id; the hook itself checks
    /// that this router is that pool's creator, so a wrong id reverts. It succeeds only while the pending fees are already in ETH —
    /// converting the token-side fees is a swap that Pons reserves for its own sweep operator. Follow with `harvest()`.
    function sweepPool(bytes32 poolId) external {
        hook.sweepPoolFees(poolId, 0, 0);
    }

    /// @notice Permissionless. Moves this router's whole balance of an ERC-20 to the registry's treasury (docs/phase1.md F1e).
    /// By the ordinary path the router never holds a token: Pons converts pool fees to ETH before crediting them. The one path that
    /// hands it tokens is Pons's owner-only `rescuePoolFees`. There is deliberately NO swap here: a permissionless swap lets its
    /// caller pick the minimum and sandwich it. What this does NOT enforce: that the treasury sells what it receives and returns
    /// the ETH (by `escrow.credit(router)` or a plain transfer, either of which `harvest()` splits 80/20). That is the treasury's word.
    function forwardToken(IERC20 token) external returns (uint256 amount) {
        if (coin == address(0)) revert NotInitialised();
        amount = token.balanceOf(address(this));
        if (amount == 0) return 0;
        address to = registry.treasury();
        token.safeTransfer(to, amount);
        emit TokenForwarded(address(token), to, amount);
    }

    /// @notice Permissionless. Claims, splits, sends. A treasury that refuses ETH never blocks the vault's share.
    function harvest() external returns (uint256 toVault, uint256 toTreasury) {
        if (coin == address(0)) revert NotInitialised();
        // docs/phase3.md L2: the treasury is called with ETH and may be any contract. It could not take anything by re-entering
        // (the books already balance when it is called), but there is no reason to let it try.
        if (_entered == 1) revert Reentered();
        _entered = 1;
        // Pons keeps a curve's fees inside the curve until somebody entitled sweeps them, and this router — the creator-fee
        // recipient — is entitled. After graduation the curve refuses (graduation swept it already); that is not an error here.
        try curve.sweepFees(0) {} catch {}
        uint256 claimed;
        if (escrow.balanceOf(address(this)) != 0) claimed = escrow.claim();

        uint256 fresh = address(this).balance - treasuryOwed;
        toVault = (fresh * VAULT_BPS) / BPS;
        toTreasury = fresh - toVault + treasuryOwed;
        if (toVault != 0) vault.deposit{value: toVault}(coin);
        if (toTreasury != 0) {
            treasuryOwed = 0;                                   // effects before the call
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
