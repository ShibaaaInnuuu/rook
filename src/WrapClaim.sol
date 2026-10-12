// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {MerkleProof} from "@openzeppelin/contracts/utils/cryptography/MerkleProof.sol";
import {ListingRegistry} from "./ListingRegistry.sol";
import {Distributor} from "./Distributor.sol";
import {Wrapper} from "./Wrapper.sol";
import {IEtchVault} from "./interfaces/IEtchVault.sol";
import {IRandomnessBeacon} from "./interfaces/IRandomnessBeacon.sol";
import {Wait} from "./lib/Wait.sol";

/// @title WrapClaim
/// @notice Step 2 of docs/v3/wrapper.md: the `distribution` address every wrapped frog's `ONE_ITEM` is minted to. It hands a
/// frog's tokens to the holders of the coin that paid for it, pro rata, by a Merkle root the keeper publishes — the path the
/// Distributor takes for rune units, minus everything that path needs because runes live on Bitcoin: a claim here transfers
/// the tokens and is finished.
/// @dev It deploys its own `Wrapper`, passing itself as `distribution`, so the two point at each other by construction and
/// neither has a setter. Being the wrapper's `deployer`, it forwards the deployer-only wiring (`wireFloorModule`,
/// `initMintLimit`, `proposeMintLimit`) from the registry's owner. What this does NOT protect against, stated as the
/// Distributor states it:
///  - THE HOLDER SNAPSHOT IS THE KEEPER'S WORD. A root is published with its list and cannot be used for 15 minutes, during
///    which anyone can recompute it and the owner can cancel it. No contract rejects a wrong root; `OverTotal` only stops a
///    root from paying out more than its own frog's tokens.
///  - A coin that never buys again leaves its carried tokens here, idle and fully backed. Deliberately no carry-only root: it
///    would be a keeper power to publish a root with nothing behind it.
///  - A token somebody sends here unasked is stranded: it is never counted, so it can never be claimed twice.
contract WrapClaim {
    ListingRegistry public immutable registry;
    uint256 public immutable listingId;
    Wrapper public immutable wrapper;

    uint256 public constant ONE_ITEM = 1e18;
    uint256 public constant CHALLENGE = 15 minutes;      // the Distributor's
    uint256 public constant CLAIM_PERIOD = 30 days;      // the Distributor's

    struct Distribution {
        bool open;
        bool rolled;
        address coin;
        uint64 challengeEnds;
        uint64 challengeEndsBlock;
        uint64 claimsEnd;
        uint64 claimsEndBlock;
        bytes32 root;
        uint256 total;       // one frog's ONE_ITEM, plus whatever the coin's earlier distributions left unclaimed
        uint256 claimed;
        string uri;
    }

    mapping(uint256 acquisitionId => Distribution) private _distributions;
    mapping(uint256 acquisitionId => mapping(address holder => bool)) public claimed;
    /// @notice Unclaimed tokens waiting for the same coin's next frog.
    mapping(address coin => uint256) public carry;
    /// @notice Wrapped frogs whose tokens are still waiting for a root: each is one `ONE_ITEM` of this contract's balance.
    uint256 public unopened;
    /// @notice Every frog this contract has seen wrapped, so `unopened` can be counted without trusting a caller.
    mapping(uint256 acquisitionId => bool) public counted;

    event Counted(uint256 indexed acquisitionId);
    event Opened(uint256 indexed acquisitionId, address indexed coin, bytes32 root, uint256 total, string uri, uint64 challengeEnds);
    event Cancelled(uint256 indexed acquisitionId);
    event Claimed(uint256 indexed acquisitionId, address indexed holder, uint256 amount);
    event RolledOver(uint256 indexed acquisitionId, address indexed coin, uint256 amount);

    error NotKeeper();
    error NotOwner();
    error NotWrapped();
    error WrongStage();
    error BadInput();
    error ChallengeWindow(uint64 until);
    error WindowOver();
    error ClaimsClosed();
    error TooEarly();
    error AlreadyClaimed();
    error BadProof();
    error OverTotal();

    /// @notice The parent chain's block time on this chain (lib/Wait.sol); its wrapper gets the same.
    uint256 public immutable parentBlockSeconds;

    constructor(ListingRegistry registry_, Distributor distributor_, IRandomnessBeacon beacon_, uint256 listingId_,
        address custodian_, string memory custodyAddress_, string memory name_, string memory symbol_, uint256 parentBlockSeconds_)
    {
        Wait.checkBlockSeconds(parentBlockSeconds_);
        parentBlockSeconds = parentBlockSeconds_;
        registry = registry_;
        listingId = listingId_;
        wrapper = new Wrapper(registry_, distributor_, beacon_, listingId_, address(this), custodian_, custodyAddress_, name_, symbol_,
            parentBlockSeconds_);
    }

    modifier onlyKeeper() { if (msg.sender != registry.keeper()) revert NotKeeper(); _; }
    modifier onlyOwner() { if (msg.sender != registry.owner()) revert NotOwner(); _; }

    // ---- the wrapper's deployer-only calls, forwarded from the registry's owner ---------------------------------------------------
    // The wrapper's `deployer` is this contract (its constructor built the wrapper), and `Wrapper.wireFloorModule`,
    // `initMintLimit` and `proposeMintLimit` are the deployer's. Without these three, a WrapClaim-built wrapper could never
    // take a floor module or have its mint rate set (docs/v6/deploy-stack.md §1, §8). The owner is read live from the registry,
    // so a rotation of the timelock moves this power with it, as it moves `cancel`. The wrapper's own checks still apply (once,
    // before anything was minted; a raise waits a week on both clocks); `acceptMintLimit` stays public on the wrapper.

    /// @notice The registry's owner: the wrapper's one floor module, once, before it has ever minted or burned anything.
    function wireFloorModule(address module) external onlyOwner {
        wrapper.wireFloorModule(module);
    }

    /// @notice The registry's owner: the wrapper's daily mint limit, once, before anything was ever minted (H11).
    function initMintLimit(uint256 limit) external onlyOwner {
        wrapper.initMintLimit(limit);
    }

    /// @notice The registry's owner: lower the wrapper's daily mint limit at once, or propose a raise that anyone may accept
    /// on the wrapper after `Wrapper.LIMIT_DELAY` on both clocks.
    function proposeMintLimit(uint256 limit) external onlyOwner {
        wrapper.proposeMintLimit(limit);
    }

    /// @dev A wrapped frog's tokens are counted as unopened the first time this contract looks at it. `wrap` mints straight to
    /// this address and calls nothing here, so the count is taken lazily — from the wrapper's own record, never a caller's word.
    function _count(uint256 acquisitionId) private {
        if (counted[acquisitionId]) return;
        if (!wrapper.wrapped(acquisitionId)) revert NotWrapped();
        counted[acquisitionId] = true;
        unopened++;
        emit Counted(acquisitionId);
    }

    /// @notice Anyone: record that a frog was wrapped, so its tokens show as unopened before the keeper publishes a root.
    /// Opening a frog counts it too; this exists so the books can be complete at any moment, not only after a root.
    function count(uint256 acquisitionId) external {
        _count(acquisitionId);
    }

    // ---- publishing a frog's holders ------------------------------------------------------------------------------------------

    /// @notice The keeper publishes who gets how much of one wrapped frog: `root` over leaves (holder, amount), `uri` where the
    /// whole list is. The total is the chain's, not the keeper's: one `ONE_ITEM` plus the coin's carry.
    function open(uint256 acquisitionId, bytes32 root, string calldata uri) external onlyKeeper {
        if (root == bytes32(0) || bytes(uri).length == 0) revert BadInput();
        Distribution storage d = _distributions[acquisitionId];
        if (d.open || d.rolled) revert WrongStage();
        _count(acquisitionId);
        address coin = IEtchVault(registry.vaultOf(listingId)).acquisition(acquisitionId).coin;
        unopened--;
        d.open = true;
        d.coin = coin;
        d.root = root;
        d.uri = uri;
        d.total = ONE_ITEM + carry[coin];
        carry[coin] = 0;
        d.challengeEnds = uint64(block.timestamp + CHALLENGE);
        d.challengeEndsBlock = uint64(block.number + Wait.blocksFor(CHALLENGE, parentBlockSeconds));
        d.claimsEnd = uint64(block.timestamp + CHALLENGE + CLAIM_PERIOD);
        d.claimsEndBlock = uint64(block.number + Wait.blocksFor(CHALLENGE + CLAIM_PERIOD, parentBlockSeconds));
        emit Opened(acquisitionId, coin, root, d.total, uri, d.challengeEnds);
    }

    /// @notice The owner, inside the 15 minutes only: a wrong root is withdrawn, the carried part goes back to the coin's carry,
    /// and the frog is unopened again so the keeper can publish a corrected one. After the 15 minutes the owner has no power
    /// over a root that anyone can already claim against.
    function cancel(uint256 acquisitionId) external onlyOwner {
        Distribution storage d = _distributions[acquisitionId];
        if (!d.open) revert WrongStage();
        if (Wait.elapsed(d.challengeEnds, d.challengeEndsBlock)) revert WindowOver();
        carry[d.coin] += d.total - ONE_ITEM;
        unopened++;
        delete _distributions[acquisitionId];
        emit Cancelled(acquisitionId);
    }

    // ---- claiming ---------------------------------------------------------------------------------------------------------------

    /// @notice Anyone may submit a holder's proof. The tokens go to the holder in the leaf, never to the caller.
    function claim(uint256 acquisitionId, address holder, uint256 amount, bytes32[] calldata proof) external {
        Distribution storage d = _distributions[acquisitionId];
        if (!d.open) revert WrongStage();
        if (!Wait.elapsed(d.challengeEnds, d.challengeEndsBlock)) revert ChallengeWindow(d.challengeEnds);
        if (d.rolled || Wait.elapsed(d.claimsEnd, d.claimsEndBlock)) revert ClaimsClosed();
        if (holder == address(0) || amount == 0) revert BadInput();
        if (claimed[acquisitionId][holder]) revert AlreadyClaimed();
        if (!MerkleProof.verifyCalldata(proof, d.root, leaf(holder, amount))) revert BadProof();
        if (d.claimed + amount > d.total) revert OverTotal();       // a root that promises more than one frog cannot pay it
        claimed[acquisitionId][holder] = true;
        d.claimed += amount;
        emit Claimed(acquisitionId, holder, amount);
        bool ok = wrapper.transfer(holder, amount);
        assert(ok);                                                 // OpenZeppelin's ERC20 reverts rather than return false
    }

    /// @notice The Distributor's `shareLeaf`, so one tool builds both kinds of root.
    function leaf(address holder, uint256 amount) public pure returns (bytes32) {
        return keccak256(bytes.concat(keccak256(abi.encode(holder, amount))));
    }

    /// @notice Anyone, once claims have closed on both clocks: what nobody claimed waits for the same coin's next frog.
    function rollOver(uint256 acquisitionId) external {
        Distribution storage d = _distributions[acquisitionId];
        if (!d.open || d.rolled) revert WrongStage();
        if (!Wait.elapsed(d.claimsEnd, d.claimsEndBlock)) revert TooEarly();
        d.rolled = true;
        uint256 left = d.total - d.claimed;
        carry[d.coin] += left;
        emit RolledOver(acquisitionId, d.coin, left);
    }

    // ---- views ------------------------------------------------------------------------------------------------------------------

    function distribution(uint256 acquisitionId) external view returns (Distribution memory) { return _distributions[acquisitionId]; }
}
