// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {ListingRegistry} from "../../src/ListingRegistry.sol";
import {NftRegistry} from "../../src/nft/NftRegistry.sol";
import {NftSweepVault} from "../../src/nft/NftSweepVault.sol";
import {NftRemoteVault} from "../../src/nft/NftRemoteVault.sol";
import {NftVaultBase} from "../../src/nft/NftVaultBase.sol";
import {Policy} from "../../src/nft/INft.sol";
import {MockLauncher, MockBeacon} from "../utils/Mocks.sol";
import {Merkle} from "../utils/Merkle.sol";
import {Clock} from "../utils/Clock.sol";
import {MockNft, MockMarket} from "./NftMocks.sol";
import {
    ItemType, OrderType, BasicOrderType, OfferItem, ConsiderationItem, OrderParameters, AdvancedOrder, CriteriaResolver,
    FulfillmentComponent, BasicOrderParameters, ISeaport16Sweep
} from "../../src/nft/Seaport16.sol";

/// Four NFT listings over one ListingRegistry's roles:
///   0  this chain, Raffle   (NftSweepVault)       2  chain 1, Raffle     (NftRemoteVault, bought by `canister`)
///   1  this chain, Burn     (NftSweepVault)       3  chain 8453, Burn    (NftRemoteVault)
/// `market` is the allow-listed sweep target (Seaport's stand-in). Coins A and B launch on listing 0, C on 1, D on 2, E on 3.
abstract contract NftBase is Test {
    ListingRegistry lr;
    NftRegistry nreg;
    MockLauncher launcher;
    MockBeacon beacon;
    MockNft nft;
    MockMarket market;
    NftSweepVault raffleVault;
    NftSweepVault burnVault;
    NftRemoteVault remoteRaffle;
    NftRemoteVault remoteBurn;

    address owner = makeAddr("owner");
    address treasury = makeAddr("treasury");
    address keeper = makeAddr("keeper");
    address purchaser = makeAddr("purchaser");
    address attestor = makeAddr("attestor");
    address payable canister = payable(makeAddr("canister (threshold key)"));
    address router = makeAddr("router");
    address coinA = makeAddr("coinA");
    address coinB = makeAddr("coinB");
    address coinC = makeAddr("coinC");
    address coinD = makeAddr("coinD");
    address coinE = makeAddr("coinE");
    address seller = makeAddr("seller");
    address sweeper = makeAddr("sweeper");
    address stranger = makeAddr("stranger");
    address remoteCollection = makeAddr("a collection on another chain");

    uint256 constant CAP = 0.1 ether;

    function setUp() public virtual {
        vm.warp(1_800_000_000);
        vm.roll(26_000_000);
        vm.startPrank(owner);
        lr = new ListingRegistry(owner, treasury);
        lr.proposeRoles(keeper, purchaser, attestor);
        beacon = new MockBeacon();
        nft = new MockNft();
        market = new MockMarket(nft);
        nreg = new NftRegistry(lr, beacon, address(market));
        launcher = new MockLauncher();
        nreg.wire(address(launcher));
        lr.wireNft(address(nreg));
        raffleVault = NftSweepVault(payable(nreg.vaultOf(nreg.listNft(uint64(block.chainid), address(nft), Policy.Raffle, "Floor", payable(0), 0))));
        burnVault = NftSweepVault(payable(nreg.vaultOf(nreg.listNft(uint64(block.chainid), address(nft), Policy.Burn, "Floor (burn)", payable(0), 0))));
        remoteRaffle = NftRemoteVault(nreg.vaultOf(nreg.listNft(1, remoteCollection, Policy.Raffle, "Eth floor", canister, 5 ether)));
        remoteBurn = NftRemoteVault(nreg.vaultOf(nreg.listNft(8453, remoteCollection, Policy.Burn, "Base floor (burn)", canister, 5 ether)));
        vm.stopPrank();
        launcher.set(coinA, router, makeAddr("creatorA"), 0);
        launcher.set(coinB, router, makeAddr("creatorB"), 0);
        launcher.set(coinC, router, makeAddr("creatorC"), 1);
        launcher.set(coinD, router, makeAddr("creatorD"), 2);
        launcher.set(coinE, router, makeAddr("creatorE"), 3);
        vm.deal(router, 10_000 ether);
        vm.prank(seller);
        nft.setApprovalForAll(address(market), true);
    }

    // ---- helpers ----------------------------------------------------------------------------------------------------------

    function _fund(NftVaultBase v, address coin, uint256 amount) internal {
        vm.prank(router);
        v.deposit{value: amount}(coin);
    }

    /// The keeper posts `capWei`. H12 bounds a post while the last cap is fresh (±20%, ten minutes apart); suites about other
    /// things post what they need, so a fresh cap is let lapse first when the new one is not a legal step
    /// (test/floor/CapStep.t.sol tests the bound itself).
    function _cap(NftVaultBase v, uint256 capWei) internal {
        if (v.capFresh()) {
            uint256 c = v.capWei();
            uint256 at = uint256(v.capExpiry()) - 1 hours;
            bool soon = vm.getBlockTimestamp() < at + 10 minutes;
            bool jump = capWei * 10 > c * 12 || capWei * 10 < c * 8;
            if (soon || jump) {
                vm.warp(uint256(v.capExpiry()) + 1);
                vm.roll(uint256(v.capExpiryBlock()) + 1);
            }
        }
        vm.prank(keeper);
        v.postCap(capWei, "https://example.invalid/floor");
    }

    /// Who sells each id, and for how much: the tests' own copy, so building an order makes no external call (an external call
    /// between `vm.expectRevert` and the sweep would be the call the expectation applies to).
    mapping(uint256 id => address) internal askSeller;
    mapping(uint256 id => uint256) internal askPrice;

    /// Mints `ids` to the seller and lists each at `price`.
    function _ask(uint256[] memory ids, uint256 price) internal {
        for (uint256 i = 0; i < ids.length; ++i) {
            nft.mint(seller, ids[i]);
            vm.prank(seller);
            market.list(ids[i], price);
            _note(ids[i], seller, price);
        }
    }

    function _note(uint256 id, address who, uint256 price) internal {
        askSeller[id] = who;
        askPrice[id] = price;
    }

    function _ids(uint256 a) internal pure returns (uint256[] memory r) { r = new uint256[](1); r[0] = a; }
    function _ids(uint256 a, uint256 b) internal pure returns (uint256[] memory r) { r = new uint256[](2); r[0] = a; r[1] = b; }
    function _ids(uint256 a, uint256 b, uint256 c) internal pure returns (uint256[] memory r) {
        r = new uint256[](3); r[0] = a; r[1] = b; r[2] = c;
    }

    // ---- Seaport 1.6 calldata, as a sweeper would build it (docs/v4/nft.md §5.1) ------------------------------------------

    /// A FULL_OPEN order: `offerer` sells `id` of `collection` for `price` wei, paid to itself.
    function _orderFor(address collection, address offerer, uint256 id, uint256 price) internal pure returns (AdvancedOrder memory a) {
        OfferItem[] memory offer = new OfferItem[](1);
        offer[0] = OfferItem(ItemType.ERC721, collection, id, 1, 1);
        ConsiderationItem[] memory cons = new ConsiderationItem[](1);
        cons[0] = ConsiderationItem(ItemType.NATIVE, address(0), 0, price, price, payable(offerer));
        a.parameters = OrderParameters(offerer, address(0), offer, cons, OrderType.FULL_OPEN, 0, type(uint256).max, bytes32(0),
            id, bytes32(0), 1);
        a.numerator = 1;
        a.denominator = 1;
    }

    function _order(address offerer, uint256 id, uint256 price) internal view returns (AdvancedOrder memory) {
        return _orderFor(address(nft), offerer, id, price);
    }

    function _available(AdvancedOrder[] memory orders, address recipient) internal pure returns (bytes memory) {
        return abi.encodeCall(ISeaport16Sweep.fulfillAvailableAdvancedOrders, (orders, new CriteriaResolver[](0),
            new FulfillmentComponent[][](0), new FulfillmentComponent[][](0), bytes32(0), recipient, orders.length));
    }

    /// fulfillAvailableAdvancedOrders for the listed asks of `ids`, one order each, bought into `recipient`.
    function _buyCall(uint256[] memory ids, address recipient) internal view returns (bytes memory) {
        AdvancedOrder[] memory orders = new AdvancedOrder[](ids.length);
        for (uint256 i = 0; i < ids.length; ++i) {
            orders[i] = _order(askSeller[ids[i]], ids[i], askPrice[ids[i]]);
        }
        return _available(orders, recipient);
    }

    /// A basic ETH-for-ERC-721 order for the listed ask of `id`.
    function _basicParams(uint256 id) internal view returns (BasicOrderParameters memory p) {
        uint256 price = askPrice[id];
        p.offerer = payable(askSeller[id]);
        p.offerToken = address(nft);
        p.offerIdentifier = id;
        p.offerAmount = 1;
        p.considerationAmount = price;
        p.basicOrderType = BasicOrderType.ETH_TO_ERC721_FULL_OPEN;
        p.endTime = type(uint256).max;
        p.salt = id;
    }

    /// A funded, capped vault sweeps `ids` at `price` each. Returns the first new acquisition id.
    function _sweep(NftSweepVault v, uint256[] memory ids, uint256 price) internal returns (uint256 first) {
        first = v.acquisitionCount();
        _ask(ids, price);
        vm.prank(sweeper);
        v.sweep(address(market), _buyCall(ids, address(v)), ids, ids.length);
    }

    // a three-holder ticket list: alice [0,50), bob [50,80), carol [80,100)
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    address carol = makeAddr("carol");

    function _tickets() internal view returns (bytes32[] memory leaves) {
        leaves = new bytes32[](3);
        leaves[0] = raffleVault.ticketLeaf(alice, 0, 50);
        leaves[1] = raffleVault.ticketLeaf(bob, 50, 80);
        leaves[2] = raffleVault.ticketLeaf(carol, 80, 100);
    }

    function _holderOf(uint256 ticket) internal view returns (address who, uint256 first, uint256 end, uint256 index) {
        if (ticket < 50) return (alice, 0, 50, 0);
        if (ticket < 80) return (bob, 50, 80, 1);
        return (carol, 80, 100, 2);
    }

    /// Opens, waits out the challenge, pins, resolves (with `randomness`) and claims. Returns the winner.
    function _raffle(NftVaultBase v, uint256 acqId, bytes32 randomness) internal returns (address who) {
        bytes32[] memory leaves = _tickets();
        vm.prank(keeper);
        v.openDraw(acqId, Merkle.root(leaves), 100, "ipfs://tickets");
        Clock.skip(15 minutes);
        v.requestDraw(acqId);
        Clock.skip(2 minutes);
        v.resolve(acqId, abi.encode(randomness));
        (who,,,) = _holderOf(v.lot(acqId).winningTicket);
        (, uint256 first, uint256 end, uint256 index) = _holderOf(v.lot(acqId).winningTicket);
        vm.prank(stranger);
        v.claimWin(acqId, who, first, end, Merkle.proof(leaves, index));
    }
}
