// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {NftBase} from "./NftBase.sol";
import {MockNft, MockMarket} from "./NftMocks.sol";
import {NftSweepVault} from "../../src/nft/NftSweepVault.sol";
import {NftVaultBase} from "../../src/nft/NftVaultBase.sol";
import {MockLauncher} from "../utils/Mocks.sol";
import {
    ItemType, OrderType, OfferItem, ConsiderationItem, OrderParameters, AdvancedOrder, CriteriaResolver, FulfillmentComponent,
    ISeaport16Sweep
} from "../../src/nft/Seaport16.sol";

/// Drives a Burn listing's sweep vault with deposits, caps, honest and dishonest sweeps (overpriced asks, tips, extra ids,
/// swallowed ETH, kept change, partial fills), burns and forced ETH, and remembers what it saw.
contract SweepHandler is Test {
    NftSweepVault public immutable vault;
    MockNft public immutable nft;
    MockMarket public immutable market;
    address public immutable keeper;
    address public immutable router;
    address[2] public coins;
    address public immutable seller = makeAddr("h-seller");
    address public immutable tipTo = makeAddr("h-tipTo");

    uint256 public nextId = 1_000;
    uint256 public ghostSwept;           // what successful sweeps spent, as the vault measured it
    uint256 public ghostBought;
    uint256 public ghostBurned;
    uint256 public ghostOverCap;         // successful sweeps that spent more than cap × bought (must stay 0)
    uint256 public ghostForced;

    constructor(NftSweepVault v, MockNft n, MockMarket m, address keeper_, address router_, address c0, address c1) {
        vault = v; nft = n; market = m; keeper = keeper_; router = router_; coins[0] = c0; coins[1] = c1;
        vm.prank(seller);
        nft.setApprovalForAll(address(market), true);
    }

    function deposit(uint256 which, uint256 amount) external {
        amount = bound(amount, 1, 3 ether);
        vm.deal(router, amount);
        vm.prank(router);
        vault.deposit{value: amount}(coins[which % 2]);
    }

    function postCap(uint256 capWei) external {
        capWei = bound(capWei, 0.001 ether, 1 ether);
        if (vault.capFresh()) {
            // H12: while fresh, a step within ±20% at least ten minutes after the last post (anything else would revert)
            uint256 c = vault.capWei();
            capWei = bound(capWei, c - c / 5, c + c / 5);
            vm.warp(vm.getBlockTimestamp() + 10 minutes);
            vm.roll(vm.getBlockNumber() + 10 minutes / 12 + 1);
        }
        vm.prank(keeper);
        vault.postCap(capWei, "h");
    }

    function sweep(uint256 count, uint256 price, uint256 tip, uint256 mode) external {
        count = bound(count, 1, 4);
        price = bound(price, 1, 1.5 ether);
        uint256[] memory ids = new uint256[](count);
        for (uint256 i = 0; i < count; ++i) {
            ids[i] = nextId++;
            nft.mint(seller, ids[i]);
            vm.prank(seller);
            market.list(ids[i], price);
        }
        mode %= 5;
        AdvancedOrder[] memory orders = new AdvancedOrder[](count);
        for (uint256 i = 0; i < count; ++i) orders[i] = _order(ids[i], price);
        if (mode == 1) {                                  // a fulfiller tip past the signed count: refused by the decoder
            ConsiderationItem[] memory c = new ConsiderationItem[](2);
            c[0] = orders[0].parameters.consideration[0];
            c[1] = ConsiderationItem(ItemType.NATIVE, address(0), 0, bound(tip, 1, 1 ether), bound(tip, 1, 1 ether), payable(tipTo));
            orders[0].parameters.consideration = c;
        } else if (mode == 2) {
            market.setMode(MockMarket.Mode.Swallow, address(0), 0);
        } else if (mode == 3) {
            uint256 extra = nextId++;
            nft.mint(seller, extra);
            market.setMode(MockMarket.Mode.PlusExtra, address(0), extra);
        } else if (mode == 4) {
            market.setMode(MockMarket.Mode.KeepChange, address(0), 0);
        }
        bytes memory data = abi.encodeCall(ISeaport16Sweep.fulfillAvailableAdvancedOrders, (orders, new CriteriaResolver[](0),
            new FulfillmentComponent[][](0), new FulfillmentComponent[][](0), bytes32(0), address(vault), (tip % count) + 1));
        uint256 cap = vault.capWei();
        try vault.sweep(address(market), data, ids, 1) returns (uint256 bought, uint256 spent) {
            ghostSwept += spent;
            ghostBought += bought;
            if (spent > cap * bought) ghostOverCap++;
        } catch {}
        market.setMode(MockMarket.Mode.Honest, address(0), 0);
    }

    function _order(uint256 id, uint256 price) internal view returns (AdvancedOrder memory a) {
        OfferItem[] memory offer = new OfferItem[](1);
        offer[0] = OfferItem(ItemType.ERC721, address(nft), id, 1, 1);
        ConsiderationItem[] memory cons = new ConsiderationItem[](1);
        cons[0] = ConsiderationItem(ItemType.NATIVE, address(0), 0, price, price, payable(seller));
        a.parameters = OrderParameters(seller, address(0), offer, cons, OrderType.FULL_OPEN, 0, type(uint256).max, bytes32(0), id,
            bytes32(0), 1);
        a.numerator = 1;
        a.denominator = 1;
    }

    function burn(uint256 seed) external {
        uint256 n = vault.acquisitionCount();
        if (n == 0) return;
        try vault.burn(seed % n) { ghostBurned++; } catch {}
    }

    function force(uint256 amount) external {
        amount = bound(amount, 1, 1 ether);
        vm.deal(address(vault), address(vault).balance + amount);
        ghostForced += amount;
    }

    function wait(uint256 secs) external {
        secs = bound(secs, 1, 2 hours);
        vm.warp(block.timestamp + secs);
        vm.roll(block.number + secs / 12 + 1);
    }
}

contract NftSweepVaultInvariantTest is NftBase {
    SweepHandler handler;
    address coinF = makeAddr("coinF");

    function setUp() public override {
        super.setUp();
        launcher.set(coinF, router, makeAddr("creatorF"), 1);
        handler = new SweepHandler(burnVault, nft, market, keeper, router, coinC, coinF);
        targetContract(address(handler));
    }

    /// The vault's balance always covers everything it owes to a purpose.
    function invariant_balanceCoversTheLedger() public view {
        assertGe(address(burnVault).balance, burnVault.accounted());
    }

    /// Every wei a coin paid in is exactly one of: spendable, waiting, or charged to a purchase that the vault measured.
    function invariant_ledgerConserves() public view {
        uint256 inflow = burnVault.contributedBy(coinC) + burnVault.contributedBy(coinF);
        uint256 charged = burnVault.chargedTo(coinC) + burnVault.chargedTo(coinF);
        assertEq(inflow, burnVault.attributed() + burnVault.waiting() + charged);
        assertEq(charged, handler.ghostSwept());
    }

    /// No successful sweep ever spent more than the cap times what arrived.
    function invariant_neverOverCap() public view {
        assertEq(handler.ghostOverCap(), 0);
    }

    /// The vault owns exactly the NFTs it bought and has not burned: nothing arrived that was not a candidate, nothing left otherwise.
    function invariant_nftsInEqualsBoughtMinusBurned() public view {
        assertEq(burnVault.acquisitionCount(), handler.ghostBought());
        assertEq(nft.balanceOf(address(burnVault)), handler.ghostBought() - handler.ghostBurned());
        assertEq(nft.balanceOf(burnVault.DEAD()), handler.ghostBurned());
    }

    /// Forced ETH is never counted: the balance is the ledger plus exactly what was forced.
    function invariant_forcedEthIsInertButNotLost() public view {
        assertEq(address(burnVault).balance, burnVault.accounted() + handler.ghostForced());
    }
}
