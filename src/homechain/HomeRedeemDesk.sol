// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {IWrappedCollection} from "../interfaces/IWrappedCollection.sol";
import {IArbInbox, IArbOutbox, IHomeAdapterAck} from "./IHomeChain.sol";

/// @title HomeRedeemDesk
/// @notice The home chain's side of a 4663 redeem intent (docs/v6/home-chain-vault.md §10.5), next to `HomeWrapper`. An intent
/// arrives through the rollup's Outbox from the 4663 adapter and nothing else; anyone acknowledges it down through the Inbox
/// (paying the retryable); once the adapter has dispatched the unit's wX up and the gateway has released it here, anyone
/// completes it: the wX is burned at `HomeWrapper` to burn or relist one random item, drawn and delivered there, through the guard.
/// @dev The desk holds wX only between a landing and a completion, and has no path for it but `complete`. It does not know which
/// landing belongs to which intent (the standard gateway carries no data) and does not need to: a burn and a relist have no
/// outcome for the redeemer, so any acknowledged intent may take any landed unit. Total completions never exceed total landings.
contract HomeRedeemDesk is ReentrancyGuard {
    IArbOutbox public immutable outbox;
    IArbInbox public immutable inbox;
    /// @notice The 4663 `HomeWrappedAdapter`: the only L2 sender an intent is accepted from, and the target of every acknowledgement.
    address public immutable adapter;
    IWrappedCollection public immutable wrapper;

    uint256 public constant ONE_ITEM = 1e18;
    uint256 public constant L2_GAS_LIMIT = 200_000;
    uint256 public constant L2_MAX_FEE = 1 gwei;

    enum Stage { None, Received, Acknowledged, Completed }
    struct Intent { Stage stage; uint8 kind; address owner; uint256 homeTicket; }

    mapping(uint256 id => Intent) private _intents;
    uint256 public received;
    uint256 public completed;

    event IntentReceived(uint256 indexed id, uint8 kind, address indexed owner);
    event IntentAcknowledged(uint256 indexed id, address indexed by);
    event IntentCompleted(uint256 indexed id, uint256 indexed homeTicket, address indexed by);

    error BadInput();
    error NotAdapter();
    error BadKind();
    error WrongStage();
    error NotArrived();
    error Insufficient(uint256 need, uint256 have);

    constructor(IArbOutbox outbox_, IArbInbox inbox_, address adapter_, IWrappedCollection wrapper_) {
        if (address(outbox_) == address(0) || address(inbox_) == address(0) || adapter_ == address(0) || address(wrapper_) == address(0)) {
            revert BadInput();
        }
        outbox = outbox_;
        inbox = inbox_;
        adapter = adapter_;
        wrapper = wrapper_;
    }

    /// @notice The adapter's L2→L1 message, executed by anyone through the Outbox: a unit was burned on 4663 for `kind`.
    function intent(uint256 id, uint8 kind, address owner) external {
        if (msg.sender != address(outbox) || outbox.l2ToL1Sender() != adapter) revert NotAdapter();
        if (kind > 1) revert BadKind();
        Intent storage i = _intents[id];
        if (i.stage != Stage.None) revert WrongStage();
        i.stage = Stage.Received;
        i.kind = kind;
        i.owner = owner;
        received++;
        emit IntentReceived(id, kind, owner);
    }

    /// @notice Anyone, paying the retryable's fee: tells the adapter the intent is here, so it dispatches the wX. Refunds of the
    /// retryable go to the adapter, never to a person.
    function acknowledge(uint256 id) external payable nonReentrant {
        Intent storage i = _intents[id];
        if (i.stage != Stage.Received) revert WrongStage();
        i.stage = Stage.Acknowledged;
        bytes memory data = abi.encodeCall(IHomeAdapterAck.acknowledge, (id));
        uint256 need = inbox.calculateRetryableSubmissionFee(data.length, block.basefee) + L2_GAS_LIMIT * L2_MAX_FEE;
        if (msg.value < need) revert Insufficient(need, msg.value);
        inbox.createRetryableTicket{value: msg.value}(adapter, 0, msg.value - L2_GAS_LIMIT * L2_MAX_FEE, adapter, adapter,
            L2_GAS_LIMIT, L2_MAX_FEE, data);
        emit IntentAcknowledged(id, msg.sender);
    }

    /// @notice Anyone, once a dispatched unit's wX has landed here: burn it at the wrapper for the intent's kind. One per intent.
    function complete(uint256 id) external nonReentrant returns (uint256 homeTicket) {
        Intent storage i = _intents[id];
        if (i.stage != Stage.Acknowledged) revert WrongStage();
        if (wrapper.balanceOf(address(this)) < ONE_ITEM) revert NotArrived();
        i.stage = Stage.Completed;
        homeTicket = i.kind == 0 ? wrapper.redeemToBurn() : wrapper.redeemToRelist();
        i.homeTicket = homeTicket;
        completed++;
        emit IntentCompleted(id, homeTicket, msg.sender);
    }

    function intentOf(uint256 id) external view returns (Intent memory) { return _intents[id]; }
}
