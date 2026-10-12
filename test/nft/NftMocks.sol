// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ERC721} from "@openzeppelin/contracts/token/ERC721/ERC721.sol";
import {IERC721} from "@openzeppelin/contracts/token/ERC721/IERC721.sol";
import {
    OrderParameters, AdvancedOrder, CriteriaResolver, FulfillmentComponent, BasicOrderParameters, Execution
} from "../../src/nft/Seaport16.sol";

/// A plain collection anyone can mint from, with an optional hook that runs on every transfer — the way a malicious or merely
/// creative collection could call back into whoever is moving its tokens.
contract MockNft is ERC721 {
    address public hookTarget;
    bytes public hookData;
    bool public hookRequireSuccess;
    constructor() ERC721("Mock Floor", "FLOOR") {}
    function mint(address to, uint256 id) external { _mint(to, id); }
    function setHook(address target, bytes calldata data, bool requireSuccess) external {
        hookTarget = target; hookData = data; hookRequireSuccess = requireSuccess;
    }
    function _update(address to, uint256 tokenId, address auth) internal override returns (address from) {
        from = super._update(to, tokenId, auth);
        address t = hookTarget;
        if (t != address(0)) {
            hookTarget = address(0);                    // once
            (bool ok, bytes memory ret) = t.call(hookData);
            if (hookRequireSuccess && !ok) assembly { revert(add(ret, 32), mload(ret)) }
        }
    }
}

/// Seaport 1.6's sweep ABI (the three entry points a sweep vault forwards), executed the way Seaport executes them where it matters to
/// the vault: every offer item moves from its offerer (who approved the market) to the fulfiller or the named recipient, every
/// consideration item is paid in ETH from what was sent, the rest is refunded to the caller, and an underpaid order reverts. It does
/// NOT check signatures: a test order needs none. `list` only remembers asks for the tests' order builders.
/// A `mode` makes it misbehave after the vault's checks have passed, so the vault's balance checks are still tested on their own.
contract MockMarket {
    struct Ask { address seller; uint256 price; bool live; }
    enum Mode { Honest, Swallow, Relay, PlusExtra, KeepChange }
    IERC721 public immutable nft;
    mapping(uint256 => Ask) public asks;
    bool public useSafeTransfer = true;
    Mode public mode;
    address public relayTo;
    uint256 public extraId;

    constructor(IERC721 nft_) { nft = nft_; }

    function list(uint256 id, uint256 price) external { asks[id] = Ask(msg.sender, price, true); }
    function setSafe(bool s) external { useSafeTransfer = s; }
    /// Swallow: takes the ETH and delivers nothing. Relay: sends all of it to `relayTo_`. PlusExtra: also moves `extra` in.
    /// KeepChange: fills honestly but keeps what it should have refunded.
    function setMode(Mode m, address relayTo_, uint256 extra) external { mode = m; relayTo = relayTo_; extraId = extra; }

    function fulfillBasicOrder(BasicOrderParameters calldata p) external payable returns (bool) { return _basic(p); }
    function fulfillBasicOrder_efficient_6GL6yc(BasicOrderParameters calldata p) external payable returns (bool) { return _basic(p); }

    function fulfillAvailableAdvancedOrders(AdvancedOrder[] calldata orders, CriteriaResolver[] calldata, FulfillmentComponent[][] calldata,
        FulfillmentComponent[][] calldata, bytes32, address recipient, uint256 maximumFulfilled) external payable
        returns (bool[] memory available, Execution[] memory executions)
    {
        available = new bool[](orders.length);
        if (_misbehaved()) return (available, executions);
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
        _extraAndRefund(to, spent);
    }

    function _basic(BasicOrderParameters calldata p) private returns (bool) {
        if (_misbehaved()) return true;
        _move(p.offerToken, p.offerer, msg.sender, p.offerIdentifier);
        uint256 spent = p.considerationAmount;
        _pay(p.offerer, p.considerationAmount);
        for (uint256 j = 0; j < p.additionalRecipients.length; ++j) {
            spent += p.additionalRecipients[j].amount;
            _pay(p.additionalRecipients[j].recipient, p.additionalRecipients[j].amount);
        }
        _extraAndRefund(msg.sender, spent);
        return true;
    }

    function _misbehaved() private returns (bool) {
        if (mode == Mode.Swallow) return true;
        if (mode == Mode.Relay) { (bool ok,) = relayTo.call{value: msg.value}(""); require(ok, "relay"); return true; }
        return false;
    }

    function _move(address token, address from, address to, uint256 id) private {
        if (token == address(nft)) asks[id].live = false;
        if (useSafeTransfer) IERC721(token).safeTransferFrom(from, to, id);
        else IERC721(token).transferFrom(from, to, id);
    }

    function _pay(address to, uint256 amount) private {
        if (amount == 0) return;
        (bool ok,) = to.call{value: amount}("");
        require(ok, "recipient refused");
    }

    function _extraAndRefund(address to, uint256 spent) private {
        if (mode == Mode.PlusExtra) nft.transferFrom(nft.ownerOf(extraId), to, extraId);
        require(msg.value >= spent, "underpaid");
        if (msg.value > spent && mode != Mode.KeepChange) { (bool ok,) = msg.sender.call{value: msg.value - spent}(""); require(ok, "refund"); }
    }
}

/// A seller whose receive() calls back into something when it is paid.
contract ReentrantSeller {
    address public target;
    bytes public data;
    bool public armed;
    bytes public lastRevert;
    function arm(address t, bytes calldata d) external { target = t; data = d; armed = true; }
    function approve(IERC721 nft, address op) external { nft.setApprovalForAll(op, true); }
    function list(MockMarket m, uint256 id, uint256 price) external { m.list(id, price); }
    receive() external payable {
        if (!armed) return;
        armed = false;
        (bool ok, bytes memory ret) = target.call{value: msg.value}(data);
        if (!ok) assembly { revert(add(ret, 32), mload(ret)) }
    }
}
