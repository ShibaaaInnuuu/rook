// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IERC721} from "@openzeppelin/contracts/token/ERC721/IERC721.sol";
import {MockBeacon} from "../utils/Mocks.sol";
import {Merkle} from "../utils/Merkle.sol";
import {Clock} from "../utils/Clock.sol";
import {DrandEvmnet} from "../../src/rand/DrandEvmnet.sol";
import {HomeGuard} from "../../src/homechain/HomeGuard.sol";
import {HomeWrapper} from "../../src/homechain/HomeWrapper.sol";
import {HomeVaultBase} from "../../src/homechain/HomeVaultBase.sol";
import {HomeVaultYoyo} from "../../src/homechain/HomeVaultYoyo.sol";
import {HomeVaultWrap} from "../../src/homechain/HomeVaultWrap.sol";
import {HomeVaultRaffle} from "../../src/homechain/HomeVaultRaffle.sol";
import {IRandomnessBeacon} from "../../src/interfaces/IRandomnessBeacon.sol";
import {
    ICryptoPunksMarket, IArbInbox, IArbOutbox, IL1GatewayRouter, IHomeGuard, ISeaportValidate, Order, OrderComponents
} from "../../src/homechain/IHomeChain.sol";
import {SeaportGuard} from "../../src/nft/SeaportGuard.sol";
import {
    ItemType, OrderType, OfferItem, ConsiderationItem, OrderParameters, BasicOrderParameters, AdditionalRecipient, ISeaport16Sweep
} from "../../src/nft/Seaport16.sol";

interface ISeaportFork is ISeaportValidate {
    function fulfillOrder(Order calldata order, bytes32 fulfillerConduitKey) external payable returns (bool);
    function information() external view returns (string memory version, bytes32 domainSeparator, address conduitController);
}

/// Ethereum mainnet, forked at block 26,040,025 (docs/v6/home-chain-vault.md §7): the real CryptoPunksMarket with punk #2346
/// offered to anyone for 34 ETH, the real Seaport 1.6 and the real BAYC, the real Inbox and L1GatewayRouter of Robinhood Chain. The
/// BAYC listing is the one that was filled for real one block later (tx 0x958bbdef…, #1304 for 8.2 ETH); its recorded calldata is
/// replayed through the vault. Nothing is sent to any public chain.
/// Run:  anvil --fork-url <ethereum rpc> --fork-block-number 26040025 --port 8641
///       forge test --match-path "test/homechain/HomeVault.fork.t.sol" --fork-url http://127.0.0.1:8641
contract HomeVaultForkTest is Test {
    ICryptoPunksMarket constant PUNKS = ICryptoPunksMarket(0xb47e3cd837dDF8e4c57F05d70Ab865de6e193BBB);
    IERC721 constant BAYC = IERC721(0xBC4CA0EdA7647A8aB7C2061c2E118A18a936f13D);
    ISeaportFork constant SEAPORT = ISeaportFork(0x0000000000000068F116a894984e2DB1123eB395);
    IArbInbox constant INBOX = IArbInbox(0x1A07cc4BD17E0118BdB54D70990D2158AbAD7a2D);
    IArbOutbox constant OUTBOX = IArbOutbox(0xf0ce991ea4A0d2400A4AB49b20ae333f6Dce3DE9);
    IL1GatewayRouter constant ROUTER = IL1GatewayRouter(0x6a2E3a1e16FC29f27Ce61429746D558d656975bB);
    address constant L1_ERC20_GATEWAY = 0x85001CC4867C5e1C22dA4B79BB8852B9e2a06da0;
    uint256 constant PUNK = 2346;
    uint256 constant PUNK_ASK = 34 ether;
    uint256 constant APE = 1304;
    uint256 constant APE_PRICE = 8.2 ether;

    bool forked;
    DrandEvmnet drand;
    MockBeacon beacon;
    HomeGuard guard;
    HomeVaultYoyo punkYoyo;
    HomeVaultYoyo apeYoyo;
    HomeVaultWrap apeWrap;
    HomeVaultRaffle apeRaffle;
    HomeWrapper wApe;
    address timelock = makeAddr("timelock");
    address keeper = makeAddr("keeper");
    address escrow = makeAddr("FeeEscrow on 4663");
    address receiver = makeAddr("HomeReceiver on 4663");
    address trigger = makeAddr("trigger");
    address funder = makeAddr("funder (a fast bridge solver)");
    address buyer = makeAddr("fork buyer");
    bytes apeFill;

    function setUp() public {
        forked = block.chainid == 1;
        if (!forked) return;
        assertEq(block.number, 26_040_025, "fork at the block before the real fill");
        drand = new DrandEvmnet();
        beacon = new MockBeacon();
        guard = new HomeGuard(timelock, 7 days, 12);
        address wrapAt = vm.computeCreateAddress(address(this), vm.getNonce(address(this)) + 3);
        wApe = new HomeWrapper(IRandomnessBeacon(address(drand)), address(BAYC), false, IHomeGuard(address(guard)), wrapAt, 12, "Wrapped Ape", "wAPE");
        punkYoyo = new HomeVaultYoyo(_config(address(PUNKS), true), 12_000, 30 days);
        apeYoyo = new HomeVaultYoyo(_config(address(BAYC), false), 12_000, 30 days);
        apeWrap = new HomeVaultWrap(_config(address(BAYC), false), address(wApe), ROUTER);
        assertEq(address(apeWrap), wrapAt);
        apeRaffle = new HomeVaultRaffle(_config(address(BAYC), false));
        vm.startPrank(timelock);
        guard.register(address(PUNKS), address(punkYoyo));
        guard.register(address(BAYC), address(apeYoyo));
        guard.register(address(BAYC), address(apeWrap));
        guard.register(address(BAYC), address(apeRaffle));
        guard.register(address(BAYC), address(wApe));
        guard.raise(address(PUNKS), 3, 40 ether, 40 ether);      // one ×4 raise from stage 0: room for a 34 ETH punk
        guard.raise(address(BAYC), 3, 40 ether, 40 ether);
        guard.raiseGlobal(12, 40 ether);
        guard.setLimits(6, 12, 40 ether);
        vm.stopPrank();
        vm.deal(funder, 200 ether);
        vm.deal(buyer, 200 ether);
        string memory json = vm.readFile("test/fixtures/bayc-1304-seaport-fill.json");
        bytes memory recorded = vm.parseJsonBytes(json, ".input");
        assertEq(vm.parseJsonUint(json, ".fillBlock"), 26_040_026);
        assertEq(vm.parseJsonUint(json, ".valueWei"), APE_PRICE);
        assertEq(bytes4(recorded), ISeaport16Sweep.fulfillBasicOrder_efficient_6GL6yc.selector);
        // the recorded parameters, exactly, except the ORIGINAL FILLER's conduit key: the fulfiller's own field, outside the offerer's
        // signature, which the vault (a fulfiller with no conduit) does not use and the guard refuses (docs/v4/nft.md §5.1)
        BasicOrderParameters memory o = abi.decode(_args(recorded), (BasicOrderParameters));
        assertTrue(o.fulfillerConduitKey != bytes32(0), "the real filler used a conduit");
        assertEq(uint8(o.basicOrderType), 0, "ETH for an ERC-721, full, open");
        assertEq(o.offerer, 0x224af408b93C6C3442B251F8b90Bf86A24145192);
        assertEq(o.offerToken, address(BAYC));
        assertEq(o.offerIdentifier, APE);
        assertEq(o.considerationAmount, 8.118 ether);
        assertEq(o.additionalRecipients.length, 1);
        assertEq(o.additionalRecipients[0].amount, 0.082 ether);
        assertGt(o.endTime, block.timestamp, "still live at the fork block");
        o.fulfillerConduitKey = bytes32(0);
        apeFill = abi.encodeCall(ISeaport16Sweep.fulfillBasicOrder_efficient_6GL6yc, (o));
    }

    function _config(address collection, bool isPunks) internal view returns (HomeVaultBase.Config memory c) {
        c.collection = collection;
        c.isPunks = isPunks;
        c.seaport = isPunks ? address(0) : address(SEAPORT);
        c.beacon = IRandomnessBeacon(address(beacon));
        c.guard = IHomeGuard(address(guard));
        c.inbox = INBOX;
        c.outbox = OUTBOX;
        c.escrow = escrow;
        c.receiver = receiver;
        c.owner = timelock;
        c.keeper = keeper;
        c.reward = 0.01 ether;
        c.buffer = 0.02 ether;
        c.parentBlockSeconds = 12;
    }

    function _fund(HomeVaultBase v, uint256 amount) internal {
        vm.prank(funder);
        v.fund{value: amount}(1);
    }

    function _cap(HomeVaultBase v, uint256 c) internal {
        vm.prank(keeper);
        v.postCap(c, "fork");
    }

    // ---- the real punks order book ----------------------------------------------------------------------------------------------

    /// Anyone buys punk #2346 at its real 34 ETH ask; it is relisted at 40.8 ETH in the same call; a fork buyer takes the ask; anyone
    /// settles: the real market's `withdraw()` pays the vault under its 2,300-gas stipend, and the proceeds go down through the REAL
    /// Inbox as a retryable to the 4663 receiver.
    function test_fork_punk_buyRelistSellSettle() public {
        if (!forked) return;
        (bool forSale,, address seller, uint256 minValue, address only) = PUNKS.punksOfferedForSale(PUNK);
        assertTrue(forSale && only == address(0), "the real offer, to anyone");
        assertEq(minValue, PUNK_ASK);
        // the 40.8 ETH ask leaves as value out at settle: above stage 1's 40 ETH day, so one more x4 raise after the interval
        Clock.skip(3 days + 1);
        vm.prank(timelock);
        guard.setLimits(6, 12, 160 ether);
        _fund(punkYoyo, 40 ether);
        _cap(punkYoyo, 35 ether);
        uint256 before = trigger.balance;
        vm.prank(trigger, trigger);
        uint256 aid = punkYoyo.buyPunk(PUNK);
        assertEq(PUNKS.punkIndexToAddress(PUNK), address(punkYoyo), "the vault owns the punk");
        assertEq(trigger.balance - before, 0.01 ether, "the fixed reward");
        assertEq(PUNKS.pendingWithdrawals(seller), PUNK_ASK, "the real market credited the seller");
        (forSale,,, minValue, only) = PUNKS.punksOfferedForSale(PUNK);
        assertTrue(forSale && only == address(0));
        assertEq(minValue, 40.8 ether, "relisted at 1.2x on the real order book");
        assertEq(punkYoyo.attributed(), 40 ether - PUNK_ASK - 0.01 ether);

        vm.prank(buyer);
        PUNKS.buyPunk{value: 40.8 ether}(PUNK);
        assertEq(PUNKS.punkIndexToAddress(PUNK), buyer);
        assertEq(PUNKS.pendingWithdrawals(address(punkYoyo)), 40.8 ether);

        uint256 balBefore = address(punkYoyo).balance;
        vm.recordLogs();
        vm.prank(trigger, trigger);
        punkYoyo.settle(aid);
        assertEq(punkYoyo.proceedsHeld(), 0);
        assertEq(address(punkYoyo).balance, punkYoyo.attributed(), "everything but the budget left: reward + bridge");
        uint256 fee = punkYoyo.bridgeFee(36);
        assertEq(balBefore, punkYoyo.attributed(), "before the withdraw, the balance was the budget alone");
        // the real Inbox emitted its message: InboxMessageDelivered(uint256 indexed messageNum, bytes data)
        Vm.Log[] memory logs = vm.getRecordedLogs();
        bool delivered;
        for (uint256 i = 0; i < logs.length; ++i) {
            if (logs[i].emitter == address(INBOX) && logs[i].topics[0] == keccak256("InboxMessageDelivered(uint256,bytes)")) delivered = true;
        }
        assertTrue(delivered, "a retryable was created on the real Inbox");
        assertEq(uint8(punkYoyo.acquisition(aid).stage), uint8(HomeVaultBase.Stage.Sold));
        fee;
    }

    // ---- the real Seaport and the real BAYC listing ------------------------------------------------------------------------------

    /// The calldata of the real fill (block 26,040,026) replayed through the vault one block earlier: the vault decodes, checks and
    /// re-encodes it, the real Seaport fills it, BAYC #1304 arrives in the vault.
    function test_fork_bayc_realListingBoughtThroughTheVault() public {
        if (!forked) return;
        assertEq(bytes4(apeFill), ISeaport16Sweep.fulfillBasicOrder_efficient_6GL6yc.selector);
        assertEq(BAYC.ownerOf(APE), 0x224af408b93C6C3442B251F8b90Bf86A24145192, "the real offerer still owns it");
        _fund(apeYoyo, 10 ether);
        _cap(apeYoyo, 9 ether);
        uint256[] memory ids = new uint256[](1);
        ids[0] = APE;
        (, uint256 total) = apeYoyo.checkOrders(apeFill, ids);
        assertEq(total, APE_PRICE, "8.118 to the offerer + 0.082 OpenSea fee");
        uint256 before = trigger.balance;
        vm.prank(trigger, trigger);
        (uint256 bought, uint256 spent) = apeYoyo.buySeaport(apeFill, ids, 1);
        assertEq(bought, 1);
        assertEq(spent, APE_PRICE);
        assertEq(BAYC.ownerOf(APE), address(apeYoyo));
        assertEq(trigger.balance - before, 0.01 ether);
        assertEq(apeYoyo.acquisition(0).pricePaid, APE_PRICE);
    }

    /// The same real order with a tip appended (a fulfiller-added recipient past the signed count): refused by the guard, and the
    /// vault's ETH does not move.
    function test_fork_bayc_aTipOnTheRealOrderIsRefused() public {
        if (!forked) return;
        _fund(apeYoyo, 10 ether);
        _cap(apeYoyo, 9 ether);
        BasicOrderParameters memory p = abi.decode(_args(apeFill), (BasicOrderParameters));
        AdditionalRecipient[] memory withTip = new AdditionalRecipient[](p.additionalRecipients.length + 1);
        for (uint256 i = 0; i < p.additionalRecipients.length; ++i) withTip[i] = p.additionalRecipients[i];
        withTip[withTip.length - 1] = AdditionalRecipient(0.5 ether, payable(makeAddr("accomplice")));
        p.additionalRecipients = withTip;                       // totalOriginalAdditionalRecipients unchanged: a tip
        uint256[] memory ids = new uint256[](1);
        ids[0] = APE;
        vm.prank(trigger, trigger);
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.Tip.selector, 0));
        apeYoyo.buySeaport(abi.encodeCall(ISeaport16Sweep.fulfillBasicOrder_efficient_6GL6yc, (p)), ids, 1);
        assertEq(address(apeYoyo).balance, 10 ether);
        // the same tip sent to Seaport directly by a plain buyer IS paid: the attack is real, and the vault is what stops it
        vm.prank(buyer);
        ISeaport16Sweep(address(SEAPORT)).fulfillBasicOrder_efficient_6GL6yc{value: APE_PRICE + 0.5 ether}(p);
        assertEq(BAYC.ownerOf(APE), buyer);
        assertEq(makeAddr("accomplice").balance, 0.5 ether);
    }

    /// YOYO on the real Seaport: the vault validates its own order (no signature), a fork buyer fills it with `fulfillOrder`, the
    /// proceeds reach the vault, `settle` bridges them through the real Inbox.
    function test_fork_bayc_relistOnRealSeaportAndSettle() public {
        if (!forked) return;
        _fund(apeYoyo, 10 ether);
        _cap(apeYoyo, 9 ether);
        uint256[] memory ids = new uint256[](1);
        ids[0] = APE;
        vm.prank(trigger, trigger);
        apeYoyo.buySeaport(apeFill, ids, 1);
        vm.prank(trigger, trigger);
        apeYoyo.relist(0);
        HomeVaultYoyo.Listing memory l = apeYoyo.listing(0);
        assertEq(l.ask, APE_PRICE * 12_000 / 10_000);
        (bool validated, bool cancelled, uint256 filled,) = SEAPORT.getOrderStatus(l.orderHash);
        assertTrue(validated && !cancelled && filled == 0, "the real Seaport recorded the vault's order as validated");

        // rebuild the order the vault validated and fill it as a plain buyer, no signature
        Order memory o;
        OfferItem[] memory offer = new OfferItem[](1);
        offer[0] = OfferItem(ItemType.ERC721, address(BAYC), APE, 1, 1);
        ConsiderationItem[] memory cons = new ConsiderationItem[](1);
        cons[0] = ConsiderationItem(ItemType.NATIVE, address(0), 0, l.ask, l.ask, payable(address(apeYoyo)));
        o.parameters = OrderParameters(address(apeYoyo), address(0), offer, cons, OrderType.FULL_OPEN, l.listedUntil - 30 days,
            l.listedUntil, bytes32(0), uint256(keccak256(abi.encode(uint256(0), uint256(0)))), bytes32(0), 1);
        bytes32 h = SEAPORT.getOrderHash(OrderComponents(o.parameters.offerer, o.parameters.zone, offer, cons, OrderType.FULL_OPEN,
            o.parameters.startTime, o.parameters.endTime, bytes32(0), o.parameters.salt, bytes32(0), SEAPORT.getCounter(address(apeYoyo))));
        assertEq(h, l.orderHash, "the same hash the vault stored");
        vm.prank(buyer);
        SEAPORT.fulfillOrder{value: l.ask}(o, bytes32(0));
        assertEq(BAYC.ownerOf(APE), buyer, "the real Seaport moved it under the vault's approval");
        assertEq(apeYoyo.proceedsHeld(), l.ask, "paid to the vault: proceeds");

        vm.recordLogs();
        vm.prank(trigger, trigger);
        apeYoyo.settle(0);
        Vm.Log[] memory logs = vm.getRecordedLogs();
        bool delivered;
        for (uint256 i = 0; i < logs.length; ++i) {
            if (logs[i].emitter == address(INBOX) && logs[i].topics[0] == keccak256("InboxMessageDelivered(uint256,bytes)")) delivered = true;
        }
        assertTrue(delivered);
        assertEq(apeYoyo.proceedsHeld(), 0);
    }

    /// Wrap on the real gateway: the ape is wrapped, and the fresh wAPE ERC-20 goes down through the REAL L1GatewayRouter (standard
    /// gateway, no registration) with its report through the real Inbox. The 4663 twin's address is deterministic before any deposit.
    function test_fork_bayc_wrapAndBridgeThroughTheRealGateway() public {
        if (!forked) return;
        assertEq(ROUTER.getGateway(address(wApe)), L1_ERC20_GATEWAY, "an unregistered token uses the standard gateway");
        address twin = ROUTER.calculateL2TokenAddress(address(wApe));
        assertTrue(twin != address(0));
        _fund(apeWrap, 10 ether);
        _cap(apeWrap, 9 ether);
        uint256[] memory ids = new uint256[](1);
        ids[0] = APE;
        vm.recordLogs();
        vm.prank(trigger, trigger);
        apeWrap.buySeaport(apeFill, ids, 1);
        assertEq(BAYC.ownerOf(APE), address(wApe), "the wrapper holds the ape");
        assertEq(wApe.held(), 1);
        assertEq(wApe.balanceOf(L1_ERC20_GATEWAY), 1e18, "the wAPE sits in the real gateway's escrow");
        assertEq(wApe.balanceOf(address(apeWrap)), 0);
        Vm.Log[] memory logs = vm.getRecordedLogs();
        uint256 inboxMessages;
        bool depositInitiated;
        for (uint256 i = 0; i < logs.length; ++i) {
            if (logs[i].emitter == address(INBOX) && logs[i].topics[0] == keccak256("InboxMessageDelivered(uint256,bytes)")) inboxMessages++;
            if (logs[i].emitter == L1_ERC20_GATEWAY && logs[i].topics[0] == keccak256("DepositInitiated(address,address,address,uint256,uint256)")) depositInitiated = true;
        }
        assertEq(inboxMessages, 2, "the token deposit's retryable and the report's");
        assertTrue(depositInitiated, "the real gateway emitted DepositInitiated");
        assertEq(uint8(apeWrap.acquisition(0).stage), uint8(HomeVaultBase.Stage.Wrapped));
    }

    /// Raffle on the fork: the real ape to a proven winner, in the claim.
    function test_fork_bayc_raffleDeliversToTheWinner() public {
        if (!forked) return;
        _fund(apeRaffle, 10 ether);
        _cap(apeRaffle, 9 ether);
        uint256[] memory ids = new uint256[](1);
        ids[0] = APE;
        vm.prank(trigger, trigger);
        apeRaffle.buySeaport(apeFill, ids, 1);
        address alice = makeAddr("alice");
        address bob = makeAddr("bob");
        bytes32[] memory leaves = new bytes32[](2);
        leaves[0] = apeRaffle.ticketLeaf(alice, 0, 50);
        leaves[1] = apeRaffle.ticketLeaf(bob, 50, 100);
        bytes32 root = Merkle.root(leaves);
        vm.prank(keeper);
        apeRaffle.openDraw(0, root, 100, "ipfs://tickets");
        Clock.skip(15 minutes);
        apeRaffle.requestDraw(0);
        Clock.skip(2 minutes);
        apeRaffle.resolve(0, abi.encode(keccak256("fork")));
        uint256 w = apeRaffle.draw(0).winningTicket;
        (address who, uint256 f, uint256 e, uint256 i) = w < 50 ? (alice, uint256(0), uint256(50), uint256(0)) : (bob, uint256(50), uint256(100), uint256(1));
        apeRaffle.claimWin(0, who, f, e, Merkle.proof(leaves, i));
        assertEq(BAYC.ownerOf(APE), who);
    }

    function _args(bytes memory data) private pure returns (bytes memory out) {
        out = new bytes(data.length - 4);
        for (uint256 i = 4; i < data.length; ++i) out[i - 4] = data[i];
    }
}

import {Vm} from "forge-std/Vm.sol";
