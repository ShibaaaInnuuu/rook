// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {GoLiveChecks} from "../script/GoLiveChecks.sol";
import {Deploy} from "../script/Deploy.s.sol";

contract GoLiveHarness {
    function checkList(GoLiveChecks.ListPlan memory p) external pure { GoLiveChecks.checkList(p); }
    function checkWrapper(GoLiveChecks.WrapperPlan memory p) external pure { GoLiveChecks.checkWrapper(p); }
    function buyerConfirm(uint256 c, address b) external pure returns (string memory) { return GoLiveChecks.expectedBuyerConfirm(c, b); }
    function wrapperConfirm(uint256 c, address d) external pure returns (string memory) { return GoLiveChecks.expectedWrapperConfirm(c, d); }
}

/// docs/deploy/contracts-mainnet.md: every refusal the two go-live steps after script/Deploy.s.sol make, one test each, plus the
/// EIP-7702 finding from the mainnet-fork dry run.
contract GoLiveChecksTest is Test {
    GoLiveHarness h;
    address deployer = 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266;
    address buyer = address(0xB0B0E1);
    address registry = address(0x4E61);
    address distributor = address(0xD157);
    address beacon = address(0xBEAC);

    function setUp() public { h = new GoLiveHarness(); }

    function _list(uint256 chainId) internal view returns (GoLiveChecks.ListPlan memory p) {
        p = GoLiveChecks.ListPlan({chainId: chainId, registry: registry, registryHasCode: true, rune: false, name: "Bitcoin Frogs",
            ref: "bitcoin-frogs", root: keccak256("root"), divisibility: 0, btcVault: "bc1p-vault", buyer: buyer, buyerHasCode: false,
            buyerConfirm: h.buyerConfirm(chainId, buyer), maxInFlight: 1 ether});
    }

    function _wrapper(uint256 chainId) internal view returns (GoLiveChecks.WrapperPlan memory p) {
        p = GoLiveChecks.WrapperPlan({chainId: chainId, deployer: deployer, confirm: h.wrapperConfirm(chainId, deployer),
            registry: registry, registryHasCode: true, registryDistributor: distributor, distributor: distributor,
            distributorBeacon: beacon, beacon: beacon, listingId: 3, listingAutonomous: true, listingIsRune: false,
            name: "Wrapped Bitcoin Frog", symbol: "wFROG", custodian: address(0xC0571D1A),
            custodyAddress: "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr",
            listingBtcVault: "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr"});
    }

    function test_theConfirmStringsAreTheirOwnWords() public view {
        assertEq(h.buyerConfirm(4663, buyer), "buyer-is:4663:0x0000000000000000000000000000000000b0b0e1");
        assertEq(h.wrapperConfirm(4663, deployer), "deploy-etch-wrapper:4663:0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266");
    }

    function test_aGoodListingPasses() public view {
        h.checkList(_list(4663));
        GoLiveChecks.ListPlan memory p = _list(4663);
        p.rune = true; p.root = bytes32(0); p.divisibility = 2;
        h.checkList(p);
    }

    function test_listingRefusals() public {
        GoLiveChecks.ListPlan memory p;
        p = _list(1); vm.expectRevert(abi.encodeWithSelector(GoLiveChecks.UnknownChain.selector, 1)); h.checkList(p);
        p = _list(4663); p.registryHasCode = false; vm.expectRevert(GoLiveChecks.RegistryIsNotAContract.selector); h.checkList(p);
        p = _list(4663); p.btcVault = ""; vm.expectRevert(GoLiveChecks.TextMissing.selector); h.checkList(p);
        p = _list(4663); p.root = bytes32(0); vm.expectRevert(GoLiveChecks.RootMissing.selector); h.checkList(p);
        p = _list(4663); p.rune = true; p.divisibility = 39; vm.expectRevert(GoLiveChecks.DivisibilityTooLarge.selector); h.checkList(p);
        p = _list(4663); p.buyer = address(0); vm.expectRevert(GoLiveChecks.BuyerMissing.selector); h.checkList(p);
        p = _list(4663); p.buyerHasCode = true; vm.expectRevert(GoLiveChecks.BuyerHasCode.selector); h.checkList(p);
        p = _list(4663); p.maxInFlight = 0; vm.expectRevert(GoLiveChecks.MaxInFlightZero.selector); h.checkList(p);
    }

    /// The buyer is immutable in the vault: a confirm typed for another address, or for another chain, is refused.
    function test_theBuyerMustBeTypedForThisChainAndThisAddress() public {
        GoLiveChecks.ListPlan memory p = _list(4663);
        p.buyerConfirm = h.buyerConfirm(4663, address(0xB0B0E2));
        vm.expectRevert(abi.encodeWithSelector(GoLiveChecks.BuyerConfirmMismatch.selector, h.buyerConfirm(4663, buyer)));
        h.checkList(p);
        p.buyerConfirm = h.buyerConfirm(31337, buyer);
        vm.expectRevert(abi.encodeWithSelector(GoLiveChecks.BuyerConfirmMismatch.selector, h.buyerConfirm(4663, buyer)));
        h.checkList(p);
    }

    function test_aGoodWrapperPasses_andOurChainsAreLooser() public view {
        h.checkWrapper(_wrapper(4663));
        GoLiveChecks.WrapperPlan memory p = _wrapper(31337);
        p.listingAutonomous = false;   // the v1 demo listing, locally
        h.checkWrapper(p);
    }

    function test_wrapperRefusals() public {
        GoLiveChecks.WrapperPlan memory p;
        p = _wrapper(46630); vm.expectRevert(abi.encodeWithSelector(GoLiveChecks.UnknownChain.selector, 46630)); h.checkWrapper(p);
        p = _wrapper(4663); p.confirm = "deploy-etch:4663:0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266";   // the system's word, not this one's
        vm.expectRevert(abi.encodeWithSelector(GoLiveChecks.ConfirmMismatch.selector, h.wrapperConfirm(4663, deployer))); h.checkWrapper(p);
        p = _wrapper(4663); p.registryHasCode = false; vm.expectRevert(GoLiveChecks.RegistryIsNotAContract.selector); h.checkWrapper(p);
        p = _wrapper(4663); p.distributor = address(0xBAD);
        vm.expectRevert(abi.encodeWithSelector(GoLiveChecks.WrongDistributor.selector, distributor, address(0xBAD))); h.checkWrapper(p);
        p = _wrapper(4663); p.beacon = address(0xBAD);
        vm.expectRevert(abi.encodeWithSelector(GoLiveChecks.WrongBeacon.selector, beacon, address(0xBAD))); h.checkWrapper(p);
        p = _wrapper(4663); p.listingIsRune = true; vm.expectRevert(abi.encodeWithSelector(GoLiveChecks.ListingIsRune.selector, 3)); h.checkWrapper(p);
        p = _wrapper(4663); p.listingAutonomous = false;
        vm.expectRevert(abi.encodeWithSelector(GoLiveChecks.ListingNotAutonomous.selector, 3)); h.checkWrapper(p);
        p = _wrapper(4663); p.symbol = ""; vm.expectRevert(GoLiveChecks.TextMissing.selector); h.checkWrapper(p);
        // docs/v3/wrapper.md, "Custody on its own key"
        p = _wrapper(4663); p.custodian = address(0); vm.expectRevert(GoLiveChecks.CustodianMissing.selector); h.checkWrapper(p);
        p = _wrapper(4663); p.custodyAddress = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq";
        vm.expectRevert(GoLiveChecks.CustodyNotTaproot.selector); h.checkWrapper(p);
        p = _wrapper(4663); p.listingBtcVault = "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0";
        vm.expectRevert(abi.encodeWithSelector(GoLiveChecks.CustodyIsNotTheListingsAddress.selector, p.listingBtcVault, p.custodyAddress));
        h.checkWrapper(p);
    }

    /// EIP-7702 is live on Robinhood Chain: a single-key account with a delegation has 23 bytes of code, 0xef0100 ‖ delegate.
    /// Deploy.s.sol must not read that as a multisig. The code below is what anvil's first dev account carries on 4663.
    function test_aDelegatedAccountIsNotAMultisig() public {
        Deploy d = new Deploy();
        assertTrue(d.isDelegatedAccount(hex"ef01008a67b5020ee254ef48e3b6a04927f39baf7e408a"));
        assertFalse(d.isDelegatedAccount(hex"ef01008a67b5020ee254ef48e3b6a04927f39baf7e40"), "22 bytes is not a designator");
        assertFalse(d.isDelegatedAccount(hex"6080604052348015600e575f5ffd5b50603e80601a5f395ff3fe60806040525f5ffdfea2"), "a real contract");
        assertFalse(d.isDelegatedAccount(""), "no code at all");
        assertFalse(d.isDelegatedAccount(address(new Deploy()).code), "a contract that happens to be 23 bytes or not");
    }
}
