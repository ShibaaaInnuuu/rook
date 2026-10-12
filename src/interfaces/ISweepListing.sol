// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title ISweepListing
/// @notice What a sweep listing needs from ListingRegistry, stated for the merge with the registry branch. Not implemented on
/// wrap-sweep: ListingRegistry.sol is not edited here. docs/v3/sweep.md, "What agent B's registry must provide".
/// @dev The registry deploys every listing's vault itself, and Launcher routes a coin's fees to `vaultOf(listingId)` once, with
/// no setter. So a sweep coin's policy is fixed by its listing, and the listing must be created with a SweepVault as its vault:
///   id = next listing id;
///   vault = new SweepVault(this, id, Wrapper(wrapper), SweepVault.Policy(policy), venue, launcher, desk); // or a factory
///   _listings[id] = Listing({kind: Kind.Sweep, vault: vault, membershipRoot: 0, btcVault: "", ...});
/// SweepVault reads back `keeper()`, `owner()`, `launcher()` and `isPaused(id)`, as the other vaults do.
interface ISweepListing {
    /// @notice Owner only. `policy` is 0 (RAFFLE) or 1 (BURN) and is fixed forever; `venue` is the only contract a pool sweep may
    /// call (or zero); `launcher` names each coin's fee router — the Pons Launcher, or the CurveFactory for coins on our own curve
    /// (docs/v3/curve.md), which then `wire`s the two vaults; `desk` is the public MintDesk (or zero).
    function listSweep(string calldata name, string calldata slug, address wrapper, uint8 policy, address venue, address launcher,
        address desk) external returns (uint256 id);
}
