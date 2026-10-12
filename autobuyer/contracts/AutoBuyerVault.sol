// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title AutoBuyerVault — a SKETCH for etched v2 (docs/v2/autonomous-buyer.md). Not wired into the v1 contracts, not audited,
///        not for deployment. It states the smallest rule set a vault needs when the purchaser is a program whose key nobody
///        holds: ETH leaves only to ONE address fixed at construction, only after a public wait, only within a cap and an
///        in-flight limit; it comes back by anyone's call; and a purchase left unsettled past its deadline stops the vault.
/// @dev What this does NOT protect against: a bug in the buyer program, a bridge that never delivers, a venue that lies about
///      what a UTXO carries, a wrong price cap from the keeper. It bounds each of those to `maxInFlight`.
contract AutoBuyerVault {
    enum State { None, Announced, Pulled, Settled, Returned, WrittenOff }

    struct Purchase {
        State state;
        uint96 amount;        // wei released (or to be)
        uint40 readyAt;       // when the announced pull may execute
        uint40 deadline;      // when an unsettled pull becomes overdue
        bool late;            // settled after its deadline
        bytes32 btcTxid;      // receipt, as posted by the buyer
        bytes32 assetId;      // inscription id hash or rune id
        uint128 assetAmount;  // 1 for an inscription; rune units otherwise
    }

    uint256 public constant PUBLIC_WAIT = 1 hours;      // v1's wait, unchanged
    uint256 public constant SETTLE_WINDOW = 3 days;     // bridge (~30 min measured) + confirmations + delivery, with room

    address public immutable owner;        // may pause and write off; can never receive or redirect funds
    address public immutable keeper;       // posts the cap (a bound on spend, not a way to move money)
    address payable public immutable buyer; // the autonomous buyer's own EVM address. Fixed for the life of the vault.
    uint256 public immutable maxInFlight;  // total wei that may be out and unsettled at once

    uint256 public capWei;                 // most one purchase may release
    uint40 public capExpiry;
    bool public paused;
    uint256 public inFlight;
    uint256 public nextId = 1;
    uint256 public overdueCount;           // > 0 stops new announcements
    mapping(uint256 => Purchase) public purchases;
    mapping(uint256 => bool) public flaggedOverdue;

    event CapPosted(uint256 capWei, uint40 expiry);
    event Announced(uint256 indexed id, uint256 amount, uint40 readyAt);
    event Pulled(uint256 indexed id, uint256 amount, uint40 deadline);
    event Settled(uint256 indexed id, bytes32 btcTxid, bytes32 assetId, uint128 assetAmount, bool late);
    event Returned(uint256 indexed id, uint256 amount);
    event Overdue(uint256 indexed id);
    event WrittenOff(uint256 indexed id, uint256 amount);
    event Paused(bool paused);

    error NotBuyer(); error NotOwner(); error NotKeeper(); error IsPaused(); error CapExpired(); error OverCap();
    error OverInFlight(); error VaultFrozen(); error WrongState(); error TooEarly(); error NotOverdue(); error WrongAmount(); error SendFailed();

    constructor(address owner_, address keeper_, address payable buyer_, uint256 maxInFlight_) {
        require(owner_ != address(0) && keeper_ != address(0) && buyer_ != address(0) && maxInFlight_ > 0, "zero");
        require(buyer_ != owner_ && buyer_ != keeper_, "buyer must be its own address");
        owner = owner_; keeper = keeper_; buyer = buyer_; maxInFlight = maxInFlight_;
    }

    receive() external payable {}

    // ---- keeper and owner: bounds and brakes only -------------------------------------------------------------------------
    function postCap(uint256 capWei_) external {
        if (msg.sender != keeper) revert NotKeeper();
        capWei = capWei_; capExpiry = uint40(block.timestamp + 1 hours);
        emit CapPosted(capWei_, capExpiry);
    }
    function setPaused(bool p) external { if (msg.sender != owner) revert NotOwner(); paused = p; emit Paused(p); }

    // ---- the buyer: announce in public, then pull to ITSELF ---------------------------------------------------------------
    function announce(uint96 amount) external returns (uint256 id) {
        if (msg.sender != buyer) revert NotBuyer();
        if (paused) revert IsPaused();
        if (overdueCount != 0) revert VaultFrozen();
        if (block.timestamp > capExpiry) revert CapExpired();
        if (amount == 0 || amount > capWei) revert OverCap();
        if (inFlight + amount > maxInFlight || amount > address(this).balance - inFlight) revert OverInFlight();
        id = nextId++;
        inFlight += amount;                                   // reserved from the moment it is announced
        purchases[id] = Purchase(State.Announced, amount, uint40(block.timestamp + PUBLIC_WAIT), 0, false, 0, 0, 0);
        emit Announced(id, amount, uint40(block.timestamp + PUBLIC_WAIT));
    }

    /// The ONLY function that sends ETH out for a purchase, and it can only send to `buyer`.
    function pull(uint256 id) external {
        if (msg.sender != buyer) revert NotBuyer();
        if (paused) revert IsPaused();
        Purchase storage p = purchases[id];
        if (p.state != State.Announced) revert WrongState();
        if (block.timestamp < p.readyAt) revert TooEarly();
        p.state = State.Pulled; p.deadline = uint40(block.timestamp + SETTLE_WINDOW);
        emit Pulled(id, p.amount, p.deadline);
        (bool ok,) = buyer.call{value: p.amount}("");
        if (!ok) revert SendFailed();
    }

    /// The receipt. Accepted late too (flagged), because a delivery that did happen should be recorded, not erased.
    function settle(uint256 id, bytes32 btcTxid, bytes32 assetId, uint128 assetAmount) external {
        if (msg.sender != buyer) revert NotBuyer();
        Purchase storage p = purchases[id];
        if (p.state != State.Pulled) revert WrongState();
        p.state = State.Settled; p.btcTxid = btcTxid; p.assetId = assetId; p.assetAmount = assetAmount;
        p.late = block.timestamp > p.deadline;
        inFlight -= p.amount;
        _clearOverdue(id);
        emit Settled(id, btcTxid, assetId, assetAmount, p.late);
    }

    /// Money that never left 4663 comes back. ANYONE may send it (the buyer program exposes a public refund that signs this
    /// call); the full amount, once. Also cancels an announcement that was never pulled (buyer only, nothing to return).
    function returnFunds(uint256 id) external payable {
        Purchase storage p = purchases[id];
        if (p.state == State.Announced) {
            if (msg.sender != buyer && msg.sender != owner) revert NotBuyer();
            if (msg.value != 0) revert WrongAmount();
        } else if (p.state == State.Pulled) {
            if (msg.value != p.amount) revert WrongAmount();
        } else revert WrongState();
        p.state = State.Returned;
        inFlight -= p.amount;
        _clearOverdue(id);
        emit Returned(id, msg.value);
    }

    // ---- anyone: say out loud that a purchase is overdue; this freezes new announcements ------------------------------------
    function flagOverdue(uint256 id) external {
        Purchase storage p = purchases[id];
        if (p.state != State.Pulled || block.timestamp <= p.deadline) revert NotOverdue();
        if (!flaggedOverdue[id]) { flaggedOverdue[id] = true; overdueCount++; emit Overdue(id); }
    }

    /// The owner's only remedy for money that went over a bridge and never arrived: record the loss, for ever, and unfreeze.
    function writeOff(uint256 id) external {
        if (msg.sender != owner) revert NotOwner();
        Purchase storage p = purchases[id];
        if (p.state != State.Pulled || !flaggedOverdue[id]) revert NotOverdue();
        p.state = State.WrittenOff;
        inFlight -= p.amount;
        _clearOverdue(id);
        emit WrittenOff(id, p.amount);
    }

    function _clearOverdue(uint256 id) private { if (flaggedOverdue[id]) { flaggedOverdue[id] = false; overdueCount--; } }
}
