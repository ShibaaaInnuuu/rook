// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC721} from "@openzeppelin/contracts/token/ERC721/IERC721.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {IRandomnessBeacon} from "../interfaces/IRandomnessBeacon.sol";
import {Wait} from "../lib/Wait.sol";
import {SeaportGuard} from "../nft/SeaportGuard.sol";
import {HomePolicy, ICryptoPunksMarket, IArbInbox, IArbOutbox, IHomeGuard, IFeeEscrowConfirm} from "./IHomeChain.sol";

/// @title HomeVaultBase
/// @notice One NFT collection's vault ON THE COLLECTION'S OWN CHAIN, with no key anywhere: an immutable contract that holds what it
/// buys, buys only what anyone can prove is listed (CryptoPunks' on-chain order book, or a Seaport 1.6 order decoded, checked and
/// re-encoded by `SeaportGuard`), and pays whoever pulls a trigger a FIXED reward from its budget. Modelled on PunkStrategy with
/// every owner lever removed (docs/v6/home-chain-vault.md §1, §3). What differs per policy — what happens to what was bought — is
/// one concrete contract each: `HomeVaultRaffle`, `HomeVaultBurn`, `HomeVaultYoyo`, `HomeVaultWrap`. One vault holds one kind of
/// money, for ever.
///
/// Money in is a ledger (`NftVaultBase`'s, re-keyed from coins to the 4663 release tickets that paid), charged first in first out
/// for every wei that leaves: the price, the caller's reward, a bridge's fee. Sale proceeds (`proceedsHeld`) are a second counter
/// only a Yoyo vault's `settle` moves.
///
/// @dev What this does NOT protect against:
///  - a keeper cap above the floor: the bounded self-dealing of docs/v4/nft.md §10.1 — a caller can make the vault buy a real item
///    of the collection at up to the cap. The guard's per-item cap bounds the cap itself.
///  - the bridge's liveness: what is sent down lands when the sequencer includes the retryable; a failed auto-redeem can be redeemed
///    by anyone for 7 days, and a dead ticket refunds to the 4663 receiver, never to a person.
///  - ETH forced in by selfdestruct: never counted, never spendable.
abstract contract HomeVaultBase is ReentrancyGuard {
    address public immutable collection;
    bool public immutable isPunks;
    HomePolicy public immutable policy;
    address public immutable seaport;
    IRandomnessBeacon public immutable beacon;
    IHomeGuard public immutable guard;
    IArbInbox public immutable inbox;
    IArbOutbox public immutable outbox;
    address public immutable escrow;        // FeeEscrow on 4663: the L2→L1 sender of native fundings, and confirm()'s target
    address public immutable receiver;      // HomeReceiver on 4663: where anything sent down goes, and nowhere else
    address public immutable owner;         // the timelock: cancels a draw in its window, rotates the keeper with a delay
    uint256 public immutable reward;
    uint256 public immutable buffer;
    uint256 public immutable parentBlockSeconds;

    uint256 public constant CAP_TTL = 1 hours;
    uint256 public constant MIN_ENTRY = 0.01 ether;
    uint256 public constant MAX_PAYERS = 64;
    uint256 public constant MAX_CANDIDATES = 50;
    uint256 public constant CHANGE_DELAY = 7 days;
    /// @notice Punks: hinted asks kept for `buyPunk`'s cheaper-offered check (docs/v6/home-chain-vault.md §3.2). A bound on the loop.
    uint256 public constant MAX_HINTS = 8;
    uint256 public constant L2_GAS_LIMIT = 200_000;
    uint256 public constant L2_MAX_FEE = 1 gwei;
    address public constant DEAD = 0x000000000000000000000000000000000000dEaD;

    // ---- the ledger ----------------------------------------------------------------------------------------------------------
    uint256[] private _queue;
    uint256 public queueHead;
    mapping(uint256 ticket => uint256) public unspentOf;
    mapping(uint256 ticket => bool) public queued;
    uint256 public attributed;      // wei of queued tickets: spendable
    uint256 public waiting;         // wei of tickets under MIN_ENTRY: theirs, not yet spendable
    uint256 public proceedsHeld;    // Yoyo sale proceeds not yet settled
    mapping(uint256 ticket => uint256) public fundedNative;
    mapping(uint256 ticket => uint256) public fundedFast;
    mapping(uint256 ticket => bool) public acknowledged;
    struct Charge { uint256 ticket; uint256 weiCharged; uint256 assignedWei; }
    mapping(uint256 purchaseId => Charge[]) private _charges;
    uint256 public purchaseCount;

    // ---- the cap and the keeper --------------------------------------------------------------------------------------------------
    address public keeper;
    address public pendingKeeper;
    uint64 public keeperReadyAt;
    uint64 public keeperReadyAtBlock;
    uint256 public capWei;
    uint64 public capExpiry;
    uint64 public capExpiryBlock;

    // ---- what was bought --------------------------------------------------------------------------------------------------------
    enum Stage { Held, Open, Pinned, Resolved, Delivered, Burned, Listed, Sold, Wrapped }
    struct Acquisition { uint256 tokenId; uint256 fundedBy; uint256 pricePaid; uint256 purchaseId; Stage stage; uint64 at; uint64 atBlock; }
    Acquisition[] internal _acquisitions;
    address internal _buying;      // the market being called during a buy: the only sender `receive` accepts then
    uint256[] private _hints;      // punk ids anyone named as offered (§3.2); re-read from the order book at every buy

    event Funded(uint256 indexed ticket, uint256 amount, address indexed from, bool native);
    event ProceedsIn(address indexed from, uint256 amount);
    event CapPosted(uint256 capWei, uint64 expiry, string capRef);
    event KeeperProposed(address keeper, uint64 readyAt);
    event KeeperChanged(address keeper);
    event Charged(uint256 indexed purchaseId, uint256 indexed ticket, uint256 amount);
    event Bought(uint256 indexed acquisitionId, uint256 indexed tokenId, uint256 indexed fundedBy, uint256 pricePaid, address trigger);
    event Rewarded(address indexed to, uint256 amount, bytes4 trigger);
    event Acknowledged(uint256 indexed ticket, uint256 amountSeen);
    event AskHinted(uint256 indexed punkId, uint256 ask, address indexed by);

    error BadInput();
    error NotKeeper();
    error NotOwner();
    error WrongKind();
    error WrongStage();
    error ZeroAmount();
    error CapExpired();
    error Insufficient(uint256 need, uint256 have);
    error NotForSale();
    error OverCap(uint256 price, uint256 cap);
    error CheaperOffered(uint256 punkId, uint256 ask);
    error AlreadyHinted();
    error HintNotCheaper();
    error NotArrived();
    error BadCandidates();
    error DuplicateCandidate(uint256 tokenId);
    error AlreadyHeld(uint256 tokenId);
    error BuyFailed(bytes reason);
    error UnlistedArrival(uint256 balanceRise, uint256 candidatesArrived);
    error NotEnoughBought(uint256 bought, uint256 promised);
    error NothingSpent();
    error OverSpent(uint256 spent, uint256 bound);
    error NotDuringBuy();
    error NoDirectPayments();
    error TooManyPayers();
    error TooEarly();
    error NothingToAcknowledge();
    error TransferFailed();

    struct Config {
        address collection; bool isPunks; address seaport; IRandomnessBeacon beacon; IHomeGuard guard; IArbInbox inbox;
        IArbOutbox outbox; address escrow; address receiver; address owner; address keeper; uint256 reward; uint256 buffer;
        uint256 parentBlockSeconds;
    }

    constructor(Config memory c, HomePolicy policy_) {
        if (c.collection == address(0) || c.collection.code.length == 0 || address(c.beacon) == address(0)
            || address(c.guard) == address(0) || c.owner == address(0) || c.keeper == address(0) || c.reward == 0
            || c.escrow == address(0) || c.receiver == address(0) || address(c.inbox) == address(0)) revert BadInput();
        if (!c.isPunks && c.seaport == address(0)) revert BadInput();
        Wait.checkBlockSeconds(c.parentBlockSeconds);
        collection = c.collection;
        isPunks = c.isPunks;
        policy = policy_;
        seaport = c.seaport;
        beacon = c.beacon;
        guard = c.guard;
        inbox = c.inbox;
        outbox = c.outbox;
        escrow = c.escrow;
        receiver = c.receiver;
        owner = c.owner;
        keeper = c.keeper;
        reward = c.reward;
        buffer = c.buffer;
        parentBlockSeconds = c.parentBlockSeconds;
    }

    modifier onlyKeeper() { if (msg.sender != keeper) revert NotKeeper(); _; }
    modifier onlyOwner() { if (msg.sender != owner) revert NotOwner(); _; }

    // ---- money in --------------------------------------------------------------------------------------------------------------

    /// @notice Anyone funds `ticket` (a 4663 FeeEscrow release). From the Outbox executing the escrow's message it is `native`;
    /// from anyone else (a fast bridge's destination call) it is `fast`. Nobody can take it out.
    function fund(uint256 ticket) external payable {
        if (msg.value == 0) revert ZeroAmount();
        if (ticket == 0) revert BadInput();
        bool native = address(outbox) != address(0) && msg.sender == address(outbox) && outbox.l2ToL1Sender() == escrow;
        if (native) fundedNative[ticket] += msg.value; else fundedFast[ticket] += msg.value;
        _credit(ticket, msg.value);
        emit Funded(ticket, msg.value, msg.sender, native);
    }

    /// @notice During a buy: only the market being called (its refund), measured by balance, no bookkeeping. Outside one: sale
    /// proceeds from Seaport into `proceedsHeld`; from the punks market nothing is written (its `withdraw` sends a 2,300-gas
    /// `transfer`; `settle` measures the balance instead); from anyone else, budget under ticket 0 — a bridge that pays by plain
    /// transfer: unattributable, still this collection's money.
    receive() external payable {
        // first, on immutables only: the punks market pays `withdraw()` with a 2,300-gas `transfer`, which affords no storage read
        if (isPunks && msg.sender == collection) return;
        if (_buying != address(0)) {
            if (msg.sender != _buying) revert NoDirectPayments();
            return;
        }
        if (msg.sender == seaport) {
            proceedsHeld += msg.value;
            emit ProceedsIn(msg.sender, msg.value);
            return;
        }
        if (msg.value == 0) revert ZeroAmount();
        _credit(0, msg.value);
        emit Funded(0, msg.value, msg.sender, false);
    }

    /// @notice Accepted only from the listing's collection and only during a buy.
    function onERC721Received(address, address, uint256, bytes calldata) external view returns (bytes4) {
        if (_buying == address(0) || msg.sender != collection) revert NotDuringBuy();
        return this.onERC721Received.selector;
    }

    function _credit(uint256 ticket, uint256 amount) private {
        uint256 b = unspentOf[ticket] + amount;
        unspentOf[ticket] = b;
        if (queued[ticket]) {
            attributed += amount;
        } else if (b >= MIN_ENTRY) {
            queued[ticket] = true;
            _queue.push(ticket);
            attributed += b;
            waiting -= b - amount;
        } else {
            waiting += amount;
        }
    }

    // ---- the cap and the keeper ----------------------------------------------------------------------------------------------------

    function postCap(uint256 capWei_, string calldata capRef) external onlyKeeper {
        if (capWei_ == 0) revert ZeroAmount();
        capWei = capWei_;
        capExpiry = uint64(block.timestamp + CAP_TTL);
        capExpiryBlock = uint64(block.number + Wait.blocksFor(CAP_TTL, parentBlockSeconds));
        emit CapPosted(capWei_, capExpiry, capRef);
    }

    function capFresh() public view returns (bool) {
        return block.timestamp <= capExpiry && block.number <= capExpiryBlock;
    }

    /// @notice The owner (timelock) names a new keeper; it lands after CHANGE_DELAY on both clocks, by anyone's call.
    function proposeKeeper(address keeper_) external onlyOwner {
        if (keeper_ == address(0)) revert BadInput();
        pendingKeeper = keeper_;
        keeperReadyAt = uint64(block.timestamp + CHANGE_DELAY);
        keeperReadyAtBlock = uint64(block.number + Wait.blocksFor(CHANGE_DELAY, parentBlockSeconds));
        emit KeeperProposed(keeper_, keeperReadyAt);
    }

    function acceptKeeper() external {
        if (pendingKeeper == address(0)) revert WrongStage();
        if (!Wait.elapsed(keeperReadyAt, keeperReadyAtBlock)) revert TooEarly();
        keeper = pendingKeeper;
        pendingKeeper = address(0);
        emit KeeperChanged(keeper);
    }

    // ---- buying: anyone's call, the contract's decision ----------------------------------------------------------------------------

    /// @notice CryptoPunks: buys `punkId` from the on-chain order book if it is offered to anyone at most the cap, no hinted punk
    /// is offered to anyone for less right now (`hintAsk`), and the spendable budget covers price + reward + buffer (+ what the
    /// policy spends right after, e.g. Wrap's bridge fees).
    function buyPunk(uint256 punkId) external nonReentrant returns (uint256 acquisitionId) {
        if (!isPunks) revert WrongKind();
        _preBuy();
        ICryptoPunksMarket p = ICryptoPunksMarket(collection);
        uint256 minValue = _openAsk(punkId);
        if (minValue == 0) revert NotForSale();
        if (minValue > capWei) revert OverCap(minValue, capWei);
        (uint256 hintId, uint256 hintAsk_) = cheaperHint(punkId, minValue);
        if (hintAsk_ != 0) revert CheaperOffered(hintId, hintAsk_);
        uint256 extra = _extraCost();
        _requireSpendable(minValue + extra);
        guard.admit(collection, 1, minValue);
        _buying = collection;
        p.buyPunk{value: minValue}(punkId);
        _buying = address(0);
        if (p.punkIndexToAddress(punkId) != address(this)) revert NotArrived();
        uint256 purchaseId = purchaseCount++;
        _chargeFifo(purchaseId, minValue + reward + extra);
        acquisitionId = _record(punkId, _attribute(purchaseId, minValue), minValue, purchaseId);
        _afterBuy(acquisitionId);
        _payReward(this.buyPunk.selector);
    }

    // ---- the cheapest-ask hint (punks; docs/v6/home-chain-vault.md §3.2) -------------------------------------------------------------

    /// @notice Anyone names a punk offered to anyone: it takes a free slot, the slot of a hint no longer offered, or — the list
    /// full — the dearest hint's slot if it undercuts it. `buyPunk` then refuses a dearer punk while a hinted one is offered.
    function hintAsk(uint256 punkId) external {
        if (!isPunks) revert WrongKind();
        uint256 ask = _openAsk(punkId);
        if (ask == 0) revert NotForSale();
        uint256 n = _hints.length;
        uint256 slot;                   // a stale hint's slot, else the dearest hint's
        uint256 worst;
        bool stale;
        for (uint256 i = 0; i < n; ++i) {
            uint256 h = _hints[i];
            if (h == punkId) revert AlreadyHinted();
            uint256 a = _openAsk(h);
            if (a == 0) { if (!stale) { stale = true; slot = i; } }
            else if (!stale && a > worst) { slot = i; worst = a; }
        }
        if (stale) _hints[slot] = punkId;
        else if (n < MAX_HINTS) _hints.push(punkId);
        else if (ask < worst) _hints[slot] = punkId;
        else revert HintNotCheaper();
        emit AskHinted(punkId, ask, msg.sender);
    }

    /// @notice The cheapest hinted punk (other than `punkId`) offered to anyone right now for less than `ask`; zeros if none.
    function cheaperHint(uint256 punkId, uint256 ask) public view returns (uint256 hintId, uint256 hintAsk_) {
        uint256 n = _hints.length;
        for (uint256 i = 0; i < n; ++i) {
            uint256 h = _hints[i];
            if (h == punkId) continue;
            uint256 a = _openAsk(h);
            if (a != 0 && a < ask && (hintAsk_ == 0 || a < hintAsk_)) { hintId = h; hintAsk_ = a; }
        }
    }

    function hints() external view returns (uint256[] memory) { return _hints; }

    /// @dev The punk's ask if it is offered to anyone by somebody else, at a non-zero price; zero otherwise (a zero ask is not an ask).
    function _openAsk(uint256 punkId) private view returns (uint256) {
        (bool forSale,, address seller, uint256 minValue, address onlySellTo) = ICryptoPunksMarket(collection).punksOfferedForSale(punkId);
        return forSale && onlySellTo == address(0) && seller != address(this) ? minValue : 0;
    }

    /// @notice Seaport 1.6: `data` is decoded, every order checked (one listed ERC-721 among `candidates`, native ETH at most the cap,
    /// no tip, nobody paid is the caller) and re-encoded by `SeaportGuard`; then the outcome is measured as `NftSweepVault.sweep` does.
    function buySeaport(bytes calldata data, uint256[] calldata candidates, uint256 minBought)
        external nonReentrant returns (uint256 bought, uint256 spent)
    {
        if (isPunks) revert WrongKind();
        _preBuy();
        uint256 n = candidates.length;
        if (n == 0 || n > MAX_CANDIDATES || minBought == 0 || minBought > n) revert BadCandidates();
        IERC721 c = IERC721(collection);
        for (uint256 i = 0; i < n; ++i) {
            for (uint256 j = 0; j < i; ++j) if (candidates[j] == candidates[i]) revert DuplicateCandidate(candidates[i]);
            if (_ownerOf(c, candidates[i]) == address(this)) revert AlreadyHeld(candidates[i]);
        }
        (bytes memory call_, uint256 value) = this.checkOrders(data, candidates);
        _requireSpendable(value + _extraCost() * n);
        uint256 nftBefore = c.balanceOf(address(this));
        uint256 ethBefore = address(this).balance;
        _buying = seaport;
        (bool ok, bytes memory ret) = seaport.call{value: value}(call_);
        _buying = address(0);
        if (!ok) revert BuyFailed(ret);
        uint256[] memory got;
        (got, bought, spent) = _measure(candidates, nftBefore, ethBefore, value, minBought);
        _recordAll(got, bought, spent);
        _payReward(this.buySeaport.selector);
    }

    /// @notice What `buySeaport` would forward for `data`, and the most ETH it could take, under the current cap: `SeaportGuard.check`.
    /// Reverts exactly as the buy would. (Its own frame on purpose: the decoder of `fulfillAvailableAdvancedOrders` inlined into the
    /// buy does not fit the stack under via-IR.)
    function checkOrders(bytes calldata data, uint256[] calldata candidates) external view returns (bytes memory forward, uint256 total) {
        return SeaportGuard.check(data, collection, candidates, capWei);
    }

    /// @dev What arrived and what left, held to the candidates, `minBought`, the cap and `value`.
    function _measure(uint256[] calldata candidates, uint256 nftBefore, uint256 ethBefore, uint256 value, uint256 minBought)
        private view returns (uint256[] memory got, uint256 bought, uint256 spent)
    {
        IERC721 c = IERC721(collection);
        got = new uint256[](candidates.length);
        for (uint256 i = 0; i < candidates.length; ++i) if (_ownerOf(c, candidates[i]) == address(this)) got[bought++] = candidates[i];
        uint256 nftAfter = c.balanceOf(address(this));
        if (nftAfter < nftBefore || nftAfter - nftBefore != bought) revert UnlistedArrival(nftAfter - nftBefore, bought);
        if (bought < minBought) revert NotEnoughBought(bought, minBought);
        uint256 ethAfter = address(this).balance;
        if (ethAfter >= ethBefore) revert NothingSpent();
        spent = ethBefore - ethAfter;
        if (spent > capWei * bought || spent > value) revert OverSpent(spent, capWei * bought);
    }

    /// @dev Admits the purchase with the guard, charges price + reward + the policy's extra to the tickets, records each item.
    function _recordAll(uint256[] memory got, uint256 bought, uint256 spent) private {
        guard.admit(collection, bought, spent);
        uint256 purchaseId = purchaseCount++;
        _chargeFifo(purchaseId, spent + reward + _extraCost() * bought);
        uint256 price = spent / bought;
        for (uint256 i = 0; i < bought; ++i) {
            uint256 p = i == bought - 1 ? spent - price * (bought - 1) : price;
            uint256 aid = _record(got[i], _attribute(purchaseId, p), p, purchaseId);
            _afterBuy(aid);
        }
    }

    function _preBuy() private view {
        if (guard.paused()) revert WrongStage();
        if (!capFresh()) revert CapExpired();
    }

    /// @dev `attributed ≥ amount + reward + buffer`: the reward and the buffer come out of the same spendable money as the price.
    function _requireSpendable(uint256 amount) internal view {
        uint256 need = amount + reward + buffer;
        if (attributed < need) revert Insufficient(need, attributed);
    }

    /// @dev What one acquisition costs beyond its price under this policy (Wrap: the bridge). Charged to the tickets with the price.
    function _extraCost() internal view virtual returns (uint256) { return 0; }

    /// @dev The policy's immediate consequence of a purchase.
    function _afterBuy(uint256 acquisitionId) internal virtual {}

    /// @dev The reward was charged to the tickets already; this is only the payment and the outflow count.
    function _payReward(bytes4 trigger) internal {
        guard.out(collection, 0, reward);
        (bool ok,) = msg.sender.call{value: reward}("");
        if (!ok) revert TransferFailed();
        emit Rewarded(msg.sender, reward, trigger);
    }

    /// @notice What a retryable with `dataLength` bytes costs on the parent chain right now: twice the submission fee (a margin for a
    /// base fee that rises before inclusion) plus the child chain's gas.
    function bridgeFee(uint256 dataLength) public view returns (uint256) {
        return 2 * inbox.calculateRetryableSubmissionFee(dataLength, block.basefee) + L2_GAS_LIMIT * L2_MAX_FEE;
    }

    // ---- the receipt back to 4663 ------------------------------------------------------------------------------------------------

    /// @notice Anyone, paying the retryable's fee: tells the 4663 escrow, through the rollup's own inbox, how much `ticket` brought
    /// here. The escrow accepts it only from this contract's aliased address. Once per ticket; no reward.
    function acknowledge(uint256 ticket) external payable nonReentrant {
        uint256 seen = fundedNative[ticket] + fundedFast[ticket];
        if (seen == 0 || acknowledged[ticket]) revert NothingToAcknowledge();
        acknowledged[ticket] = true;
        bytes memory data = abi.encodeCall(IFeeEscrowConfirm.confirm, (ticket, seen));
        uint256 need = inbox.calculateRetryableSubmissionFee(data.length, block.basefee) + L2_GAS_LIMIT * L2_MAX_FEE;
        if (msg.value < need) revert Insufficient(need, msg.value);
        inbox.createRetryableTicket{value: msg.value}(escrow, 0, msg.value - L2_GAS_LIMIT * L2_MAX_FEE, receiver, receiver,
            L2_GAS_LIMIT, L2_MAX_FEE, data);
        emit Acknowledged(ticket, seen);
    }

    // ---- internals ------------------------------------------------------------------------------------------------------------------

    function _chargeFifo(uint256 purchaseId, uint256 amount) internal {
        uint256 left = amount;
        uint256 payers;
        Charge[] storage cs = _charges[purchaseId];
        while (left != 0) {
            if (++payers > MAX_PAYERS) revert TooManyPayers();
            if (queueHead >= _queue.length) revert Insufficient(left, 0);
            uint256 ticket = _queue[queueHead];
            uint256 b = unspentOf[ticket];
            uint256 take = b < left ? b : left;
            unspentOf[ticket] = b - take;
            left -= take;
            cs.push(Charge(ticket, take, 0));
            emit Charged(purchaseId, ticket, take);
            if (b == take) { queued[ticket] = false; queueHead++; }
        }
        attributed -= amount;
    }

    function _attribute(uint256 purchaseId, uint256 price) private returns (uint256) {
        Charge[] storage cs = _charges[purchaseId];
        uint256 best;
        uint256 bestGap;
        for (uint256 i = 0; i < cs.length; ++i) {
            uint256 gap = cs[i].weiCharged > cs[i].assignedWei ? cs[i].weiCharged - cs[i].assignedWei : 0;
            if (gap > bestGap) { bestGap = gap; best = i; }
        }
        cs[best].assignedWei += price;
        return cs[best].ticket;
    }

    function _record(uint256 tokenId, uint256 ticket, uint256 price, uint256 purchaseId) private returns (uint256 aid) {
        aid = _acquisitions.length;
        Acquisition storage a = _acquisitions.push();
        a.tokenId = tokenId;
        a.fundedBy = ticket;
        a.pricePaid = price;
        a.purchaseId = purchaseId;
        a.at = uint64(block.timestamp);
        a.atBlock = uint64(block.number);
        emit Bought(aid, tokenId, ticket, price, msg.sender);
    }

    function _send(address to, uint256 tokenId) internal {
        if (isPunks) {
            ICryptoPunksMarket(collection).transferPunk(to, tokenId);
            if (ICryptoPunksMarket(collection).punkIndexToAddress(tokenId) != to) revert TransferFailed();
        } else {
            IERC721(collection).transferFrom(address(this), to, tokenId);
        }
    }

    function _owned(uint256 tokenId) internal view returns (bool) {
        if (isPunks) return ICryptoPunksMarket(collection).punkIndexToAddress(tokenId) == address(this);
        return _ownerOf(IERC721(collection), tokenId) == address(this);
    }

    function _ownerOf(IERC721 c, uint256 tokenId) internal view returns (address) {
        try c.ownerOf(tokenId) returns (address o) { return o; } catch { return address(0); }
    }

    function _stage(Acquisition storage a, Stage s) internal {
        a.stage = s;
        a.at = uint64(block.timestamp);
        a.atBlock = uint64(block.number);
    }

    // ---- views -----------------------------------------------------------------------------------------------------------------------

    function acquisition(uint256 id) external view returns (Acquisition memory) { return _acquisitions[id]; }
    function acquisitionCount() external view returns (uint256) { return _acquisitions.length; }
    function chargesOf(uint256 purchaseId) external view returns (Charge[] memory) { return _charges[purchaseId]; }
    function queueLength() external view returns (uint256) { return _queue.length; }
    function queueAt(uint256 i) external view returns (uint256) { return _queue[i]; }
    /// @notice Everything the vault owes to a purpose. Its balance is never below this.
    function accounted() external view returns (uint256) { return attributed + waiting + proceedsHeld; }
}
