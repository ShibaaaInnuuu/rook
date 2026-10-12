// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IERC721} from "@openzeppelin/contracts/token/ERC721/IERC721.sol";
import {
    OrderParameters, AdvancedOrder, CriteriaResolver, FulfillmentComponent, BasicOrderParameters, Execution
} from "../../src/nft/Seaport16.sol";
import {Order, OrderComponents} from "../../src/homechain/IHomeChain.sol";

/// CryptoPunksMarket, the parts that matter, with the real contract's semantics: an owner offers to anyone or to one address; a
/// buyer pays at least `minValue`; the seller is CREDITED and must `withdraw()`, which pays with a 2,300-gas `transfer`.
contract MockPunks {
    struct Offer { bool isForSale; uint256 punkIndex; address seller; uint256 minValue; address onlySellTo; }
    mapping(uint256 => address) public punkIndexToAddress;
    mapping(uint256 => Offer) public punksOfferedForSale;
    mapping(address => uint256) public pendingWithdrawals;
    mapping(address => uint256) public balanceOf;
    bool public refuseTransfersTo;
    address public blocked;

    function mint(address to, uint256 id) external { require(punkIndexToAddress[id] == address(0)); punkIndexToAddress[id] = to; balanceOf[to]++; }
    function setBlocked(address who) external { blocked = who; }

    function offerPunkForSale(uint256 id, uint256 minValue) external {
        require(punkIndexToAddress[id] == msg.sender, "not owner");
        punksOfferedForSale[id] = Offer(true, id, msg.sender, minValue, address(0));
    }
    function offerPunkForSaleToAddress(uint256 id, uint256 minValue, address to) external {
        require(punkIndexToAddress[id] == msg.sender, "not owner");
        punksOfferedForSale[id] = Offer(true, id, msg.sender, minValue, to);
    }
    function punkNoLongerForSale(uint256 id) external {
        require(punkIndexToAddress[id] == msg.sender, "not owner");
        punksOfferedForSale[id] = Offer(false, id, msg.sender, 0, address(0));
    }
    function buyPunk(uint256 id) external payable {
        Offer memory o = punksOfferedForSale[id];
        require(o.isForSale, "not for sale");
        require(o.onlySellTo == address(0) || o.onlySellTo == msg.sender, "not for you");
        require(msg.value >= o.minValue, "underpaid");
        require(o.seller == punkIndexToAddress[id], "stale");
        _move(o.seller, msg.sender, id);
        pendingWithdrawals[o.seller] += msg.value;
    }
    function transferPunk(address to, uint256 id) external {
        require(punkIndexToAddress[id] == msg.sender, "not owner");
        require(to != blocked, "blocked");
        _move(msg.sender, to, id);
    }
    function withdraw() external {
        uint256 amount = pendingWithdrawals[msg.sender];
        pendingWithdrawals[msg.sender] = 0;
        payable(msg.sender).transfer(amount);
    }
    function _move(address from, address to, uint256 id) private {
        punkIndexToAddress[id] = to;
        balanceOf[from]--;
        balanceOf[to]++;
        punksOfferedForSale[id] = Offer(false, id, to, 0, address(0));
    }
}

/// Seaport 1.6 as the vault uses it: the three sweep entry points executed honestly (offer items move from the offerer to the
/// recipient, consideration is paid in ETH, the rest refunded, an underpaid order reverts), plus the validation path a YOYO vault
/// needs (`validate` by the offerer records the hash; `fill` by a buyer executes a validated order). Signatures are not checked.
contract MockSeaport {
    mapping(address => uint256) public counters;
    mapping(bytes32 => Order) private _validated;
    mapping(bytes32 => bool) public isValidated;
    mapping(bytes32 => bool) public isFilled;
    mapping(bytes32 => bool) public isCancelled;
    bool public useSafeTransfer = true;
    bool public swallow;    // takes the ETH, delivers nothing

    function setSafe(bool s) external { useSafeTransfer = s; }
    function setSwallow(bool s) external { swallow = s; }

    // ---- the sweep subset ----
    function fulfillBasicOrder(BasicOrderParameters calldata p) external payable returns (bool) { return _basic(p); }
    function fulfillBasicOrder_efficient_6GL6yc(BasicOrderParameters calldata p) external payable returns (bool) { return _basic(p); }

    function fulfillAvailableAdvancedOrders(AdvancedOrder[] calldata orders, CriteriaResolver[] calldata, FulfillmentComponent[][] calldata,
        FulfillmentComponent[][] calldata, bytes32, address recipient, uint256 maximumFulfilled) external payable
        returns (bool[] memory available, Execution[] memory executions)
    {
        available = new bool[](orders.length);
        if (swallow) return (available, executions);
        address to = recipient == address(0) ? msg.sender : recipient;
        uint256 spent;
        for (uint256 i = 0; i < orders.length && i < maximumFulfilled; ++i) {
            OrderParameters calldata o = orders[i].parameters;
            for (uint256 j = 0; j < o.offer.length; ++j) _move(o.offer[j].token, o.offerer, to, o.offer[j].identifierOrCriteria);
            for (uint256 j = 0; j < o.consideration.length; ++j) {
                spent += o.consideration[j].startAmount;
                _pay(o.consideration[j].recipient, o.consideration[j].startAmount);
            }
            available[i] = true;
        }
        _refund(spent);
    }

    function _basic(BasicOrderParameters calldata p) private returns (bool) {
        if (swallow) return true;
        _move(p.offerToken, p.offerer, msg.sender, p.offerIdentifier);
        uint256 spent = p.considerationAmount;
        _pay(p.offerer, p.considerationAmount);
        for (uint256 j = 0; j < p.additionalRecipients.length; ++j) {
            spent += p.additionalRecipients[j].amount;
            _pay(p.additionalRecipients[j].recipient, p.additionalRecipients[j].amount);
        }
        _refund(spent);
        return true;
    }

    // ---- validation: the offerer, with no signature ----
    function getCounter(address offerer) external view returns (uint256) { return counters[offerer]; }
    function incrementCounter() external returns (uint256) { return ++counters[msg.sender]; }
    function getOrderHash(OrderComponents calldata c) external pure returns (bytes32) { return keccak256(abi.encode(c)); }

    function validate(Order[] calldata orders) external returns (bool) {
        for (uint256 i = 0; i < orders.length; ++i) {
            require(orders[i].parameters.offerer == msg.sender, "mock: only the offerer validates without a signature");
            bytes32 h = _hash(orders[i].parameters, counters[msg.sender]);
            isValidated[h] = true;
            _validated[h] = orders[i];
        }
        return true;
    }

    function getOrderStatus(bytes32 h) external view returns (bool, bool, uint256, uint256) {
        return (isValidated[h], isCancelled[h], isFilled[h] ? 1 : 0, 1);
    }

    /// A buyer fills a validated order: every offer item to the buyer, every consideration item paid from `msg.value`.
    function fill(bytes32 h) external payable {
        require(isValidated[h] && !isFilled[h] && !isCancelled[h], "mock: not fillable");
        OrderParameters memory o = _validated[h].parameters;
        require(block.timestamp >= o.startTime && block.timestamp < o.endTime, "mock: expired");
        isFilled[h] = true;
        uint256 spent;
        for (uint256 j = 0; j < o.offer.length; ++j) _move(o.offer[j].token, o.offerer, msg.sender, o.offer[j].identifierOrCriteria);
        for (uint256 j = 0; j < o.consideration.length; ++j) {
            spent += o.consideration[j].startAmount;
            _pay(o.consideration[j].recipient, o.consideration[j].startAmount);
        }
        _refund(spent);
    }

    function _hash(OrderParameters memory o, uint256 counter) private pure returns (bytes32) {
        return keccak256(abi.encode(OrderComponents(o.offerer, o.zone, o.offer, o.consideration, o.orderType, o.startTime, o.endTime,
            o.zoneHash, o.salt, o.conduitKey, counter)));
    }

    function _move(address token, address from, address to, uint256 id) private {
        if (useSafeTransfer) IERC721(token).safeTransferFrom(from, to, id);
        else IERC721(token).transferFrom(from, to, id);
    }
    function _pay(address to, uint256 amount) private {
        if (amount == 0) return;
        (bool ok,) = to.call{value: amount}("");
        require(ok, "recipient refused");
    }
    function _refund(uint256 spent) private {
        require(msg.value >= spent, "underpaid");
        if (msg.value > spent) { (bool ok,) = msg.sender.call{value: msg.value - spent}(""); require(ok, "refund"); }
    }
}

/// Nitro's Inbox: the real submission-fee formula, and every retryable recorded with its ETH kept here.
contract MockInbox {
    struct Retryable {
        address to; uint256 l2CallValue; uint256 maxSubmissionCost; address excessFeeRefund; address callValueRefund;
        uint256 gasLimit; uint256 maxFeePerGas; bytes data; uint256 value; address from;
    }
    Retryable[] public tickets;
    function calculateRetryableSubmissionFee(uint256 dataLength, uint256 baseFee) external pure returns (uint256) {
        return (1400 + 6 * dataLength) * baseFee;
    }
    function createRetryableTicket(address to, uint256 l2CallValue, uint256 maxSubmissionCost, address excessFeeRefundAddress,
        address callValueRefundAddress, uint256 gasLimit, uint256 maxFeePerGas, bytes calldata data) external payable returns (uint256)
    {
        require(msg.value >= maxSubmissionCost + l2CallValue + gasLimit * maxFeePerGas, "InsufficientValue");
        tickets.push(Retryable(to, l2CallValue, maxSubmissionCost, excessFeeRefundAddress, callValueRefundAddress, gasLimit, maxFeePerGas,
            data, msg.value, msg.sender));
        return tickets.length;
    }
    function count() external view returns (uint256) { return tickets.length; }
    function last() external view returns (Retryable memory) { return tickets[tickets.length - 1]; }
}

/// The Outbox executing one L2→L1 message: `l2ToL1Sender` is set only during the call, as the real one does.
contract MockOutbox {
    address public l2ToL1Sender;
    function execute(address l2Sender, address target, bytes calldata data) external payable {
        l2ToL1Sender = l2Sender;
        (bool ok, bytes memory ret) = target.call{value: msg.value}(data);
        l2ToL1Sender = address(0);
        if (!ok) assembly { revert(add(ret, 32), mload(ret)) }
    }
}

/// ArbSys on the child chain: records every L2→L1 message and keeps its value (the real precompile burns it into the bridge).
contract MockArbSys {
    struct Sent { address destination; bytes data; uint256 value; address from; }
    Sent[] public sent;
    function sendTxToL1(address destination, bytes calldata data) external payable returns (uint256) {
        sent.push(Sent(destination, data, msg.value, msg.sender));
        return sent.length;
    }
    function count() external view returns (uint256) { return sent.length; }
    function last() external view returns (Sent memory) { return sent[sent.length - 1]; }
}

/// The L1 gateway router and its standard gateway in one: pulls the token from the sender (which approved the gateway), and requires
/// the exact retryable value, as the real gateway's inbox call does.
contract MockGatewayRouter {
    struct Deposit { address token; address to; uint256 amount; uint256 maxGas; uint256 gasPriceBid; uint256 maxSubmissionCost; uint256 value; }
    Deposit[] public deposits;
    function getGateway(address) external view returns (address) { return address(this); }
    function calculateL2TokenAddress(address l1Token) external pure returns (address) {
        return address(uint160(uint256(keccak256(abi.encode("l2", l1Token)))));
    }
    function outboundTransfer(address token, address to, uint256 amount, uint256 maxGas, uint256 gasPriceBid, bytes calldata data)
        external payable returns (bytes memory)
    {
        (uint256 maxSubmissionCost, bytes memory extra) = abi.decode(data, (uint256, bytes));
        require(extra.length == 0, "EXTRA_DATA_DISABLED");
        require(msg.value >= maxSubmissionCost + maxGas * gasPriceBid, "WRONG_ETH_VALUE");
        IERC20(token).transferFrom(msg.sender, address(this), amount);
        deposits.push(Deposit(token, to, amount, maxGas, gasPriceBid, maxSubmissionCost, msg.value));
        return "";
    }
    function count() external view returns (uint256) { return deposits.length; }
    function last() external view returns (Deposit memory) { return deposits[deposits.length - 1]; }
}

/// A caller that is a contract: the vault sees it as `msg.sender`; the EOA driving it is `tx.origin`.
contract TriggerBot {
    receive() external payable {}
    function call(address target, bytes calldata data) external returns (bytes memory ret) {
        bool ok;
        (ok, ret) = target.call(data);
        if (!ok) assembly { revert(add(ret, 32), mload(ret)) }
    }
}

/// A caller whose receive() reverts: a reward it cannot take.
contract RefusingCaller {
    function call(address target, bytes calldata data) external {
        (bool ok, bytes memory ret) = target.call(data);
        if (!ok) assembly { revert(add(ret, 32), mload(ret)) }
    }
}

/// A router that buys "coin" for ETH at a fixed rate and sends it to `to`, for the receiver's buyback tests.
contract MockBuybackRouter {
    uint256 public rate;           // coin units per wei
    address public lastTo;
    uint256 public lastEth;
    mapping(address => uint256) public burnedOf;
    constructor(uint256 rate_) { rate = rate_; }
    function setRate(uint256 r) external { rate = r; }
    function buy(address coin, uint256 minOut, address to, uint256) external payable returns (uint256 out) {
        out = msg.value * rate;
        require(out >= minOut, "TooLittle");
        lastTo = to; lastEth = msg.value;
        burnedOf[coin] += out;
    }
}

contract MockQuote {
    uint256 public rate;
    constructor(uint256 rate_) { rate = rate_; }
    function setRate(uint256 r) external { rate = r; }
    function coinOutForEth(address, uint256 ethIn) external view returns (uint256) { return ethIn * rate; }
}

contract MockERC20 {
    string public name = "wX";
    string public symbol = "wX";
    uint8 public decimals = 18;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    uint256 public totalSupply;
    function mint(address to, uint256 amount) external { balanceOf[to] += amount; totalSupply += amount; }
    function transfer(address to, uint256 amount) external returns (bool) { balanceOf[msg.sender] -= amount; balanceOf[to] += amount; return true; }
    function approve(address s, uint256 amount) external returns (bool) { allowance[msg.sender][s] = amount; return true; }
    function transferFrom(address f, address t, uint256 amount) external returns (bool) {
        allowance[f][msg.sender] -= amount; balanceOf[f] -= amount; balanceOf[t] += amount; return true;
    }
    /// `StandardArbERC20.bridgeBurn`, as the L2 gateway calls it (no allowance: the gateway's own right).
    function bridgeBurn(address from, uint256 amount) external { balanceOf[from] -= amount; totalSupply -= amount; }
}

/// The L2 gateway router and its standard gateway in one: burns the caller's twin (`bridgeBurn`) and records the withdrawal the
/// L1 gateway will release ≈ 7 days later. The standard gateway refuses extra data.
contract MockL2GatewayRouter {
    struct Withdrawal { address l1Token; address to; uint256 amount; address from; }
    MockERC20 public immutable twin;
    Withdrawal[] public withdrawals;
    bool public refuse;
    constructor(MockERC20 twin_) { twin = twin_; }
    function setRefuse(bool r) external { refuse = r; }
    function outboundTransfer(address l1Token, address to, uint256 amount, bytes calldata data) external payable returns (bytes memory) {
        require(!refuse, "gateway paused");
        require(data.length == 0, "EXTRA_DATA_DISABLED");
        twin.bridgeBurn(msg.sender, amount);
        withdrawals.push(Withdrawal(l1Token, to, amount, msg.sender));
        return "";
    }
    function count() external view returns (uint256) { return withdrawals.length; }
    function last() external view returns (Withdrawal memory) { return withdrawals[withdrawals.length - 1]; }
}
