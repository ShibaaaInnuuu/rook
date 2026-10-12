// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @notice The parts of Pons V2 this project calls, declared from its verified source (docs/phase0.md §1).
/// The fork tests are what tie these declarations to the live contracts.
interface IPonsV2LaunchFactory {
    struct Socials {
        string twitter;
        string telegram;
        string discord;
        string website;
        string farcaster;
    }

    struct TokenParams {
        string name;
        string symbol;
        string logo;
        string description;
        Socials socials;
        address creatorFeeRecipient;
        uint16 creatorTaxBps;
        bool buybackEnabled;
        bytes32 expectedEconomics;
        bytes32 salt;
    }

    function launchToken(TokenParams calldata params, uint256 launchConfigId, address pairToken, address[] calldata snipeTaxExemptions)
        external
        payable
        returns (address token, address curve);

    function launchFee() external view returns (uint256);
    function feeEscrow() external view returns (address);
    function memeHook() external view returns (address);
}

/// @notice Fees accrue INSIDE the curve and reach the escrow only when swept. Besides Pons's own sweep operator, the one address
/// allowed to sweep is the launch's current creator-fee recipient (the curve calls it `deployer`) — and only while no buyback is pending.
interface IPonsV2BondingCurve {
    function sweepFees(uint256 minBuybackTokensOut) external;
    function graduated() external view returns (bool);
}

/// @notice After graduation the pool's hook holds the fees; the same two callers may sweep, and a non-operator only when the sweep
/// needs no internal swap (fees already in the quote asset).
interface IPonsV2MemeHook {
    function sweepPoolFees(bytes32 poolId, uint256 minConversionQuoteOut, uint256 minBuybackTokensOut) external;
}

interface IPonsV2FeeEscrow {
    function claim() external returns (uint256);
    function balanceOf(address recipient) external view returns (uint256);
    function credit(address recipient) external payable;
}
