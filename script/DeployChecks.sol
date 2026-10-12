// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Strings} from "@openzeppelin/contracts/utils/Strings.sol";

/// @title DeployChecks
/// @notice docs/phase2.md §1. Every reason the deploy script refuses, in one place, with no cheatcodes, so that a test can run them.
/// A deployment is refused unless the person running it has typed out which chain and which sender they mean.
library DeployChecks {
    uint256 internal constant ROBINHOOD = 4663;
    uint256 internal constant LOCAL = 31337;
    /// docs/phase6.md: our hosted fork of Robinhood Chain. Its own chain id, so nothing signed for it can be replayed on 4663.
    /// THE one constant on the Solidity side. It equals staging/chain-id.json (test/DeployChecks.t.sol reads the file and compares).
    /// Never 4663, and never 46630, which is Robinhood's public testnet.
    uint256 internal constant STAGING = 9466301;

    struct Plan {
        uint256 chainId;          // what the RPC reports
        address deployer;         // the account that would send
        string confirm;           // ETCH_DEPLOY_CONFIRM, as typed
        address owner;            // the multisig
        address treasury;
        address keeper;
        address purchaser;
        address attestor;
        bool ownerHasCode;
        bool ponsHasCode;
        /// docs/phase6.md tier 2: a small real test on 4663. It has its own confirm string and its own deployment file, and
        /// its owner may be a plain hardware-wallet address. Meaningless, and refused, on any other chain.
        bool testLabel;
    }

    error UnknownChain(uint256 chainId);
    error ConfirmMismatch(string expected);
    error OwnerMissing();
    error OwnerIsDeployer();
    error OwnerIsNotAContract();
    error RoleMissing();
    error RolesNotDistinct();
    error RoleIsOwnerOrDeployer();
    error PonsMissing();
    error TestLabelOffRobinhood(uint256 chainId);
    error RegistryMissing();
    error RegistryIsNotAContract();
    error TokenIsNotAContract();

    /// @notice `deploy-etch:<chainid>:<deployer, lowercase hex>` — the only string that lets a deployment proceed.
    function expectedConfirm(uint256 chainId, address deployer) internal pure returns (string memory) {
        return string.concat("deploy-etch:", Strings.toString(chainId), ":", Strings.toHexString(deployer));
    }

    /// @notice `deploy-etch-test:4663:<deployer>`. A different word, so that neither string can stand in for the other: a test
    /// cannot be deployed with the real confirm, and the real thing cannot be deployed with a test's.
    function expectedTestConfirm(uint256 chainId, address deployer) internal pure returns (string memory) {
        return string.concat("deploy-etch-test:", Strings.toString(chainId), ":", Strings.toHexString(deployer));
    }

    /// @notice The parent chain's block time on `chainId` (src/lib/Wait.sol): deployment config for every contract that waits.
    /// 4663 measured 12.06–12.14 s per parent block (rounded down). Local and staging are 4663's stand-ins. Any other chain: zero,
    /// and the script refuses unless `ETCH_PARENT_BLOCK_SECONDS` names that chain's measured value.
    function parentBlockSeconds(uint256 chainId) internal pure returns (uint256) {
        if (chainId == ROBINHOOD || chainId == LOCAL || chainId == STAGING) return 12;
        return 0;
    }

    /// @notice A chain whose "multisig" is an address the node itself sends from: the developer's machine, and our staging fork.
    function isOurs(uint256 chainId) internal pure returns (bool) { return chainId == LOCAL || chainId == STAGING; }

    /// @notice Where the deployment is written. A test never writes the file the real deployment would.
    function fileFor(uint256 chainId, bool testLabel) internal pure returns (string memory) {
        return string.concat("deployments/", testLabel ? "test-" : "", Strings.toString(chainId), ".json");
    }

    /// @notice `deploy-etch-treasury:<chainid>:<deployer>`. docs/token/protocol-token-v1.md §7. A third word, so that none of the
    /// three confirm strings can stand in for another: the treasury cannot be deployed with the system's confirm, or the reverse.
    function expectedTreasuryConfirm(uint256 chainId, address deployer) internal pure returns (string memory) {
        return string.concat("deploy-etch-treasury:", Strings.toString(chainId), ":", Strings.toHexString(deployer));
    }

    /// @notice The treasury is deployed against an EXISTING registry, long after it, and owns nothing itself. So it needs less than
    /// a full deployment: the chain, the confirm string, a registry with code, and — if the token is already launched — a token with
    /// code. The token may be address(0) here: it is named later, once, by the owner (docs/token/protocol-token-v1.md §6 (b)).
    struct TreasuryPlan {
        uint256 chainId;
        address deployer;
        string confirm;
        address registry;
        address token;
        bool registryHasCode;
        bool tokenHasCode;
    }

    function checkTreasury(TreasuryPlan memory p) internal pure {
        if (p.chainId != ROBINHOOD && !isOurs(p.chainId)) revert UnknownChain(p.chainId);
        string memory expected = expectedTreasuryConfirm(p.chainId, p.deployer);
        if (keccak256(bytes(p.confirm)) != keccak256(bytes(expected))) revert ConfirmMismatch(expected);
        if (p.registry == address(0)) revert RegistryMissing();
        if (!p.registryHasCode) revert RegistryIsNotAContract();
        if (p.token != address(0) && !p.tokenHasCode) revert TokenIsNotAContract();
    }

    function check(Plan memory p) internal pure {
        if (p.chainId != ROBINHOOD && !isOurs(p.chainId)) revert UnknownChain(p.chainId);
        if (p.testLabel && p.chainId != ROBINHOOD) revert TestLabelOffRobinhood(p.chainId);
        string memory expected = p.testLabel ? expectedTestConfirm(p.chainId, p.deployer) : expectedConfirm(p.chainId, p.deployer);
        if (keccak256(bytes(p.confirm)) != keccak256(bytes(expected))) revert ConfirmMismatch(expected);
        if (p.owner == address(0)) revert OwnerMissing();
        if (p.owner == p.deployer) revert OwnerIsDeployer();
        // on a public chain the owner must be a contract: a multisig. On our own chains it is an address the node sends from, and
        // in a labelled test it may be the tester's own hardware wallet.
        if (!isOurs(p.chainId) && !p.testLabel && !p.ownerHasCode) revert OwnerIsNotAContract();
        if (p.treasury == address(0) || p.keeper == address(0) || p.purchaser == address(0) || p.attestor == address(0)) revert RoleMissing();
        if (p.keeper == p.purchaser || p.keeper == p.attestor || p.purchaser == p.attestor) revert RolesNotDistinct();
        if (_isOwnerOrDeployer(p.keeper, p) || _isOwnerOrDeployer(p.purchaser, p) || _isOwnerOrDeployer(p.attestor, p)) revert RoleIsOwnerOrDeployer();
        if (!p.ponsHasCode) revert PonsMissing();
    }

    function _isOwnerOrDeployer(address a, Plan memory p) private pure returns (bool) {
        return a == p.owner || a == p.deployer;
    }
}
