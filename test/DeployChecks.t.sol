// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {DeployChecks} from "../script/DeployChecks.sol";
import {Deploy} from "../script/Deploy.s.sol";

/// docs/phase2.md §1: every reason a deployment is refused. The library is called through this harness so a revert can be expected.
contract ChecksHarness {
    function check(DeployChecks.Plan memory p) external pure { DeployChecks.check(p); }
    function expected(uint256 chainId, address deployer) external pure returns (string memory) { return DeployChecks.expectedConfirm(chainId, deployer); }
    function expectedTest(uint256 chainId, address deployer) external pure returns (string memory) { return DeployChecks.expectedTestConfirm(chainId, deployer); }
    function fileFor(uint256 chainId, bool testLabel) external pure returns (string memory) { return DeployChecks.fileFor(chainId, testLabel); }
    function checkTreasury(DeployChecks.TreasuryPlan memory p) external pure { DeployChecks.checkTreasury(p); }
    function expectedTreasury(uint256 chainId, address deployer) external pure returns (string memory) { return DeployChecks.expectedTreasuryConfirm(chainId, deployer); }
}

contract DeployChecksTest is Test {
    ChecksHarness h;
    address deployer = 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266;
    address owner = address(0x0111);
    address treasury = address(0x0222);
    address keeper = address(0x0333);
    address purchaser = address(0x0444);
    address attestor = address(0x0555);

    function setUp() public { h = new ChecksHarness(); }

    function _good(uint256 chainId) internal view returns (DeployChecks.Plan memory p) {
        p = DeployChecks.Plan(chainId, deployer, h.expected(chainId, deployer), owner, treasury, keeper, purchaser, attestor, true, true, false);
    }

    function test_theConfirmStringIsChainAndLowercaseDeployer() public view {
        assertEq(h.expected(4663, deployer), "deploy-etch:4663:0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266");
        h.check(_good(4663));
        h.check(_good(31337));
    }

    function test_refusedWithoutTheConfirmString() public {
        DeployChecks.Plan memory p = _good(4663);
        p.confirm = "";
        vm.expectRevert(abi.encodeWithSelector(DeployChecks.ConfirmMismatch.selector, "deploy-etch:4663:0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266"));
        h.check(p);
        p.confirm = "yes";
        vm.expectRevert();
        h.check(p);
    }

    function test_refusedOnAChainIdMismatch_aLocalConfirmDoesNotDeployToRobinhood() public {
        DeployChecks.Plan memory p = _good(4663);
        p.confirm = h.expected(31337, deployer);          // typed for the local chain, run against the real one
        vm.expectRevert(abi.encodeWithSelector(DeployChecks.ConfirmMismatch.selector, "deploy-etch:4663:0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266"));
        h.check(p);
        p = _good(31337);
        p.confirm = h.expected(4663, deployer);           // and the other way round
        vm.expectRevert();
        h.check(p);
    }

    function test_refusedForAnotherDeployersConfirm() public {
        DeployChecks.Plan memory p = _good(4663);
        p.confirm = h.expected(4663, address(0xBEEF));
        vm.expectRevert();
        h.check(p);
    }

    function test_refusedOnAChainItDoesNotKnow() public {
        DeployChecks.Plan memory p = _good(1);
        vm.expectRevert(abi.encodeWithSelector(DeployChecks.UnknownChain.selector, 1));
        h.check(p);
    }

    function test_theOwnerIsNeverTheDeployer_andOnAPublicChainIsAContract() public {
        DeployChecks.Plan memory p = _good(4663);
        p.owner = address(0);
        vm.expectRevert(DeployChecks.OwnerMissing.selector); h.check(p);
        p.owner = deployer;
        vm.expectRevert(DeployChecks.OwnerIsDeployer.selector); h.check(p);
        p = _good(4663); p.ownerHasCode = false;
        vm.expectRevert(DeployChecks.OwnerIsNotAContract.selector); h.check(p);
        p = _good(31337); p.ownerHasCode = false;          // locally the owner is a dev account
        h.check(p);
    }

    function test_rolesAreDistinct_andNoneIsTheOwnerOrTheDeployer() public {
        DeployChecks.Plan memory p = _good(4663);
        p.attestor = p.purchaser;
        vm.expectRevert(DeployChecks.RolesNotDistinct.selector); h.check(p);
        p = _good(4663); p.keeper = p.attestor;
        vm.expectRevert(DeployChecks.RolesNotDistinct.selector); h.check(p);
        p = _good(4663); p.keeper = p.purchaser;
        vm.expectRevert(DeployChecks.RolesNotDistinct.selector); h.check(p);
        p = _good(4663); p.purchaser = owner;
        vm.expectRevert(DeployChecks.RoleIsOwnerOrDeployer.selector); h.check(p);
        p = _good(4663); p.keeper = deployer;
        vm.expectRevert(DeployChecks.RoleIsOwnerOrDeployer.selector); h.check(p);
        p = _good(4663); p.treasury = address(0);
        vm.expectRevert(DeployChecks.RoleMissing.selector); h.check(p);
    }

    function test_refusedWherePonsIsNotDeployed() public {
        DeployChecks.Plan memory p = _good(4663);
        p.ponsHasCode = false;
        vm.expectRevert(DeployChecks.PonsMissing.selector); h.check(p);
    }

    // ---- docs/phase6.md: the staging fork, and a small real test ----

    function test_theStagingForkIsOneOfOurs_itsOwnerNeedNotBeAContract_andItsConfirmNamesItsOwnChain() public {
        DeployChecks.Plan memory p = _good(9466301); p.ownerHasCode = false;
        h.check(p);
        assertEq(h.expected(9466301, deployer), "deploy-etch:9466301:0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266");
        p.confirm = h.expected(4663, deployer);                     // a confirm typed for Robinhood Chain does not deploy staging
        vm.expectRevert(abi.encodeWithSelector(DeployChecks.ConfirmMismatch.selector, h.expected(9466301, deployer))); h.check(p);
        p = _good(4663); p.confirm = h.expected(9466301, deployer);   // and staging's does not deploy to Robinhood Chain
        vm.expectRevert(abi.encodeWithSelector(DeployChecks.ConfirmMismatch.selector, h.expected(4663, deployer))); h.check(p);
    }

    function test_theStagingIdIsTheRepositorysOneNumber_andRobinhoodsPublicTestnetIsNotOurs() public {
        assertEq(vm.parseJsonUint(vm.readFile("staging/chain-id.json"), ".chainId"), DeployChecks.STAGING, "script/DeployChecks.sol and staging/chain-id.json disagree");
        assertTrue(DeployChecks.STAGING != 4663 && DeployChecks.STAGING != 46630);
        DeployChecks.Plan memory p = _good(46630);                  // Robinhood's public testnet reports 46630: a real network, never deployed to
        vm.expectRevert(abi.encodeWithSelector(DeployChecks.UnknownChain.selector, uint256(46630))); h.check(p);
    }

    function test_theStagingCollectionsRootIsTheOneTheOperatorBuildsProofsFor() public {
        // keeper/staging/members.ts builds the same tree; keeper/test/send-guards.test.ts pins the same value
        assertEq(new Deploy().stagingRoot(), 0xc46b9158cf27bde8deea8ab99cdb7bcf071d86c3a8ae4df8019df01f754af505);
    }

    function test_aLabelledTest_hasItsOwnConfirm_itsOwnFile_andNeitherStringStandsInForTheOther() public {
        DeployChecks.Plan memory p = _good(4663); p.testLabel = true;
        vm.expectRevert(abi.encodeWithSelector(DeployChecks.ConfirmMismatch.selector, h.expectedTest(4663, deployer))); h.check(p);   // the real confirm does not deploy a test
        p.confirm = h.expectedTest(4663, deployer);
        h.check(p);
        p.ownerHasCode = false;                                     // the tester's own hardware wallet may own a test
        h.check(p);
        p.testLabel = false;                                        // a test's confirm does not deploy the real thing
        vm.expectRevert(abi.encodeWithSelector(DeployChecks.ConfirmMismatch.selector, h.expected(4663, deployer))); h.check(p);
        assertEq(h.expectedTest(4663, deployer), "deploy-etch-test:4663:0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266");
        assertEq(h.fileFor(4663, true), "deployments/test-4663.json");
        assertEq(h.fileFor(4663, false), "deployments/4663.json");
        assertEq(h.fileFor(9466301, false), "deployments/9466301.json");
    }

    function test_aTestLabelMeansNothingOffRobinhoodChain_andTheRolesRulesStillHold() public {
        for (uint256 i = 0; i < 2; i++) {
            uint256 chain = i == 0 ? 31337 : 9466301;
            DeployChecks.Plan memory p = _good(chain); p.testLabel = true; p.confirm = h.expectedTest(chain, deployer);
            vm.expectRevert(abi.encodeWithSelector(DeployChecks.TestLabelOffRobinhood.selector, chain)); h.check(p);
        }
        DeployChecks.Plan memory q = _good(4663); q.testLabel = true; q.confirm = h.expectedTest(4663, deployer);
        q.attestor = q.purchaser;
        vm.expectRevert(DeployChecks.RolesNotDistinct.selector); h.check(q);
        q = _good(4663); q.testLabel = true; q.confirm = h.expectedTest(4663, deployer); q.owner = deployer;
        vm.expectRevert(DeployChecks.OwnerIsDeployer.selector); h.check(q);
    }

    // ---- the treasury's own deployment: docs/token/protocol-token-v1.md §7 ----

    function _goodTreasury(uint256 chainId) internal view returns (DeployChecks.TreasuryPlan memory p) {
        p = DeployChecks.TreasuryPlan(chainId, deployer, h.expectedTreasury(chainId, deployer), address(0x9999), address(0), true, false);
    }

    function test_theTreasuryHasItsOwnConfirmString_whichNeitherOfTheOthersSatisfies() public {
        assertEq(h.expectedTreasury(4663, deployer), "deploy-etch-treasury:4663:0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266");
        h.checkTreasury(_goodTreasury(4663));
        h.checkTreasury(_goodTreasury(31337));
        h.checkTreasury(_goodTreasury(9466301));

        string memory want = h.expectedTreasury(4663, deployer);
        DeployChecks.TreasuryPlan memory p = _goodTreasury(4663);
        p.confirm = h.expected(4663, deployer);                      // the system's confirm does not deploy a treasury
        vm.expectRevert(abi.encodeWithSelector(DeployChecks.ConfirmMismatch.selector, want)); h.checkTreasury(p);
        p.confirm = h.expectedTest(4663, deployer);
        vm.expectRevert(abi.encodeWithSelector(DeployChecks.ConfirmMismatch.selector, want)); h.checkTreasury(p);
        p.confirm = "";
        vm.expectRevert(abi.encodeWithSelector(DeployChecks.ConfirmMismatch.selector, want)); h.checkTreasury(p);
        // nor the other way round: a treasury confirm does not deploy the system
        DeployChecks.Plan memory q = _good(4663);
        q.confirm = h.expectedTreasury(4663, deployer);
        vm.expectRevert(abi.encodeWithSelector(DeployChecks.ConfirmMismatch.selector, h.expected(4663, deployer))); h.check(q);
    }

    function test_theConfirmNamesTheChainAndTheSender_soNeitherCanBeSwapped() public {
        DeployChecks.TreasuryPlan memory p = _goodTreasury(4663);
        p.chainId = 31337;                                            // right string, wrong chain
        vm.expectRevert(abi.encodeWithSelector(DeployChecks.ConfirmMismatch.selector, h.expectedTreasury(31337, deployer)));
        h.checkTreasury(p);
        p = _goodTreasury(4663);
        p.deployer = address(0xBEEF);                                 // right string, wrong sender
        vm.expectRevert(abi.encodeWithSelector(DeployChecks.ConfirmMismatch.selector, h.expectedTreasury(4663, address(0xBEEF))));
        h.checkTreasury(p);
    }

    function test_aTreasuryIsNotDeployedToAChainWeDoNotDeployTo() public {
        uint256[3] memory chains = [uint256(1), 46630, 8453];
        for (uint256 i = 0; i < chains.length; ++i) {
            DeployChecks.TreasuryPlan memory p = _goodTreasury(chains[i]);
            vm.expectRevert(abi.encodeWithSelector(DeployChecks.UnknownChain.selector, chains[i]));
            h.checkTreasury(p);
        }
    }

    function test_theRegistryMustExistAndHaveCode_andATokenIfNamedMustToo() public {
        DeployChecks.TreasuryPlan memory p = _goodTreasury(4663);
        p.registry = address(0);
        vm.expectRevert(DeployChecks.RegistryMissing.selector); h.checkTreasury(p);
        p = _goodTreasury(4663); p.registryHasCode = false;
        vm.expectRevert(DeployChecks.RegistryIsNotAContract.selector); h.checkTreasury(p);
        // the token may be unset: it is named later, once, by the owner (docs/token/protocol-token-v1.md §6 (b))
        p = _goodTreasury(4663); p.token = address(0); h.checkTreasury(p);
        // but a token that IS named must be a contract
        p = _goodTreasury(4663); p.token = address(0x7777); p.tokenHasCode = false;
        vm.expectRevert(DeployChecks.TokenIsNotAContract.selector); h.checkTreasury(p);
        p.tokenHasCode = true; h.checkTreasury(p);
    }
}