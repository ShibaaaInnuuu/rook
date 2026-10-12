// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {Ownable2Step} from "@openzeppelin/contracts/access/Ownable2Step.sol";
import {MerkleProof} from "@openzeppelin/contracts/utils/cryptography/MerkleProof.sol";
import {AssetVault} from "./AssetVault.sol";
import {AutoBuyerVault} from "./AutoBuyerVault.sol";
import {Wait} from "./lib/Wait.sol";

/// @title ListingRegistry
/// @notice What may be paired with a coin — an ordinals collection or a rune — and who holds which role. Each listing gets its own
/// vault, deployed here so that a listing can never point at somebody else's contract.
/// @dev What this does NOT protect against: the owner listing a collection under a misleading name or with a membership root
/// that is not the collection (an ordinals collection is off-chain metadata; the root is the owner's word about it), or naming
/// roles that collude. It makes those acts public and slow; it does not make them impossible.
contract ListingRegistry is Ownable2Step {
    enum Kind { Inscriptions, Rune }

    struct Listing {
        Kind kind;
        bool paused;
        bool autonomous;          // true: the vault is an AutoBuyerVault (v2). false: a bonded AssetVault (v1)
        uint8 divisibility;       // a rune's; 0 for inscriptions
        address vault;
        bytes32 membershipRoot;   // inscriptions only: Merkle root over inscription ids
        string name;
        string ref;               // a collection's slug, or a rune's id "block:tx"
        string btcVault;          // the Bitcoin address receipts are checked against, by people
    }

    uint256 public constant CHANGE_DELAY = 7 days;

    Listing[] private _listings;
    address public launcher;      // set once
    address public distributor;   // set once
    address public treasury;

    /// @dev The keeper as named; `keeper()` answers `address(0)` while it is paused (docs/audit/2026-09-26/fixes-B-C-keeper-draws.md §0).
    address private _keeper;
    address public purchaser;
    address public attestor;

    /// @notice Every address named as an autonomous buyer by any listing. A key that can pull a vault's ETH to itself must
    /// never also be the key that posts the cap it pulls against, or the key that can pause and write off — so no role may be
    /// rotated onto one of these, ever. Kept as a set so the check is one lookup and not a loop over every listing.
    mapping(address => bool) public isAutoBuyer;

    struct PendingRoles { address keeper; address purchaser; address attestor; uint64 readyAt; }
    struct PendingRoot { bytes32 root; uint64 readyAt; }
    struct PendingText { string value; uint64 readyAt; }

    PendingRoles public pendingRoles;
    mapping(uint256 id => PendingRoot) public pendingRoot;
    mapping(uint256 id => PendingText) private _pendingBtcVault;
    /// @notice The share of every released-and-unsettled wei that a vault's bond must cover before it releases more (docs/phase1.md F4).
    /// Raised at once; lowered only after CHANGE_DELAY in public.
    uint16 public bondBps = 10_000;
    uint16 public pendingBondBps;
    uint64 public pendingBondAt;

    address public pendingTreasury;
    uint64 public pendingTreasuryAt;
    /// @notice docs/audit/README.md "Single-clock waits": the parent chain's half of each CHANGE_DELAY above (lib/Wait.sol), so a
    /// sequencer stamping ahead cannot bring a governance change forward. Declared last and outside the pending structs, so
    /// nothing a getter returns changed shape.
    uint64 public pendingRolesReadyBlock;
    mapping(uint256 id => uint64) public pendingRootReadyBlock;
    mapping(uint256 id => uint64) public pendingBtcVaultReadyBlock;
    uint64 public pendingBondReadyBlock;
    uint64 public pendingTreasuryReadyBlock;
    /// @notice docs/v4/nft.md §2.1: the NFT registry, set once, and the only caller of `reserveBuyer`. Declared last, so nothing
    /// above moved.
    address public nftRegistry;
    /// @notice The keeper incident switch (docs/audit/2026-09-26/fixes-B-C-keeper-draws.md §0). While set, `keeper()` is zero, so
    /// every keeper power everywhere reverts, and the floor module and the sweep vaults stop every inflow priced by the keeper's
    /// word. Burns and redemptions are not affected. The owner or the guardian sets it at once; only the owner clears it.
    bool public keeperPaused;
    /// @notice A key that can only stop things: pause the keeper, and cancel a draw before it is pinned. The owner names it at
    /// once (zero: none).
    address public guardian;
    /// @notice When the owner last resumed a paused keeper, on both clocks (0: never). The floor module holds unfilled curves'
    /// refunds for a grace after it (R3 check A3-L1). Declared last, so nothing above moved.
    uint64 public keeperResumedAt;
    uint64 public keeperResumedAtBlock;

    event Listed(uint256 indexed id, Kind kind, address vault, string name, string ref);
    event AutoBuyerNamed(uint256 indexed id, address buyer, uint256 maxInFlight);
    event Paused(uint256 indexed id, bool paused);
    event RolesProposed(address keeper, address purchaser, address attestor, uint64 readyAt);
    event RolesSet(address keeper, address purchaser, address attestor);
    event RootProposed(uint256 indexed id, bytes32 root, uint64 readyAt);
    event RootSet(uint256 indexed id, bytes32 root);
    event BtcVaultProposed(uint256 indexed id, string btcVault, uint64 readyAt);
    event BtcVaultSet(uint256 indexed id, string btcVault);
    event TreasuryProposed(address treasury, uint64 readyAt);
    event TreasurySet(address treasury);
    event BondBpsProposed(uint16 bps, uint64 readyAt);
    event BondBpsSet(uint16 bps);
    event Wired(address launcher, address distributor);
    event NftRegistryWired(address nftRegistry);
    event BuyerReserved(address indexed buyer);
    event KeeperPaused(address indexed by, bool paused);
    event GuardianSet(address guardian);

    error ZeroAddress();
    error SameKey();
    error AlreadyWired();
    error NoListing();
    error NothingPending();
    error TooEarly(uint64 readyAt);
    error WrongKind();
    error Empty();
    error BadBps();
    error OwnershipIsNotRenounceable();
    error NotNftRegistry();
    error NotGuardian();

    constructor(address owner_, address treasury_) Ownable(owner_) {
        if (treasury_ == address(0)) revert ZeroAddress();
        treasury = treasury_;
        emit TreasurySet(treasury_);
    }

    /// @notice Names the Launcher and the Distributor. Once: a registry whose launcher could be swapped could re-attribute fees.
    function wire(address launcher_, address distributor_) external onlyOwner {
        if (launcher != address(0)) revert AlreadyWired();
        if (launcher_ == address(0) || distributor_ == address(0)) revert ZeroAddress();
        launcher = launcher_;
        distributor = distributor_;
        emit Wired(launcher_, distributor_);
    }

    /// @notice Names the NFT registry (docs/v4/nft.md §2.1). Once: it is the only address that may reserve a buyer here.
    function wireNft(address nftRegistry_) external onlyOwner {
        if (nftRegistry != address(0)) revert AlreadyWired();
        if (nftRegistry_ == address(0)) revert ZeroAddress();
        nftRegistry = nftRegistry_;
        emit NftRegistryWired(nftRegistry_);
    }

    /// @notice docs/v4/nft.md §2.1, §10.6. The NFT registry reserves a remote listing's buyer (the canister's address, which can
    /// pull that vault's ETH to itself) the moment it names it: from then on no role, pending or later, may be that address,
    /// exactly as for a v2 buyer. Only refuses; loosens nothing, so it needs no delay.
    function reserveBuyer(address buyer) external {
        if (msg.sender != nftRegistry || buyer == address(0)) revert NotNftRegistry();
        _requireNoRole(buyer);
        isAutoBuyer[buyer] = true;
        emit BuyerReserved(buyer);
    }

    /// @dev Ownable2Step's first step, refused onto an address some vault treats as its autonomous buyer.
    function transferOwnership(address newOwner) public override onlyOwner {
        if (isAutoBuyer[newOwner] || _isGuardian(newOwner)) revert SameKey();
        super.transferOwnership(newOwner);
    }

    /// @notice Disabled. docs/security/internal-review-v1.md H2: `setPaused` is instant by design, and correctly so, but
    /// together with an inherited `renounceOwnership` it was a one-way door — pause every listing, renounce, and no key on
    /// earth can unpause a vault that has no withdraw function. Two transactions, no delay, every vault's balance gone for
    /// ever. Ownership still moves, but only to somebody, and only through `Ownable2Step`'s two steps.
    function renounceOwnership() public pure override {
        revert OwnershipIsNotRenounceable();
    }

    // ---- listings ------------------------------------------------------------------------------------------------------------

    function listInscriptions(string calldata name, string calldata slug, bytes32 membershipRoot, string calldata btcVault)
        external onlyOwner returns (uint256 id)
    {
        if (membershipRoot == bytes32(0)) revert Empty();
        id = _list(Kind.Inscriptions, 0, membershipRoot, name, slug, btcVault);
    }

    function listRune(string calldata name, string calldata runeId, uint8 divisibility, string calldata btcVault)
        external onlyOwner returns (uint256 id)
    {
        if (divisibility > 38) revert Empty();   // the runes protocol's own maximum
        id = _list(Kind.Rune, divisibility, bytes32(0), name, runeId, btcVault);
    }

    /// @notice etched v2: a listing whose vault releases ETH only to one autonomous buyer, fixed for the life of the vault.
    /// `maxInFlight` is the most that vault may have announced-or-pulled and unsettled at once — the bound that replaces v1's
    /// bond. See `docs/v2/autonomous-buyer.md`.
    function listInscriptionsAuto(string calldata name, string calldata slug, bytes32 membershipRoot, string calldata btcVault,
        address payable buyer, uint256 maxInFlight) external onlyOwner returns (uint256 id)
    {
        if (membershipRoot == bytes32(0)) revert Empty();
        id = _listAuto(Kind.Inscriptions, 0, membershipRoot, name, slug, btcVault, buyer, maxInFlight);
    }

    function listRuneAuto(string calldata name, string calldata runeId, uint8 divisibility, string calldata btcVault,
        address payable buyer, uint256 maxInFlight) external onlyOwner returns (uint256 id)
    {
        if (divisibility > 38) revert Empty();
        id = _listAuto(Kind.Rune, divisibility, bytes32(0), name, runeId, btcVault, buyer, maxInFlight);
    }

    function _list(Kind kind, uint8 divisibility, bytes32 root, string calldata name, string calldata ref, string calldata btcVault)
        private returns (uint256 id)
    {
        _requireText(name, ref, btcVault);
        id = _listings.length;
        address vault = address(new AssetVault(this, id, kind == Kind.Rune, divisibility));
        _listings.push(Listing(kind, false, false, divisibility, vault, root, name, ref, btcVault));
        emit Listed(id, kind, vault, name, ref);
    }

    function _listAuto(Kind kind, uint8 divisibility, bytes32 root, string calldata name, string calldata ref,
        string calldata btcVault, address payable buyer, uint256 maxInFlight) private returns (uint256 id)
    {
        _requireText(name, ref, btcVault);
        if (buyer == address(0)) revert ZeroAddress();
        if (maxInFlight == 0) revert Empty();
        // the same separation the roles have, at the only moment a buyer is ever named
        _requireNoRole(buyer);
        id = _listings.length;
        address vault = address(new AutoBuyerVault(this, id, kind == Kind.Rune, divisibility, buyer, maxInFlight));
        _listings.push(Listing(kind, false, true, divisibility, vault, root, name, ref, btcVault));
        isAutoBuyer[buyer] = true;
        emit Listed(id, kind, vault, name, ref);
        emit AutoBuyerNamed(id, buyer, maxInFlight);
    }

    /// @dev A buyer may be no role that is, or is waiting to be: a pending rotation onto it would otherwise land after it was named.
    function _requireNoRole(address a) private view {
        PendingRoles storage p = pendingRoles;
        if (a == owner() || a == pendingOwner() || a == _keeper || a == purchaser || a == attestor || a == treasury
            || a == pendingTreasury || a == p.keeper || a == p.purchaser || a == p.attestor || _isGuardian(a)) revert SameKey();
    }

    function _isGuardian(address a) private view returns (bool) {
        return a != address(0) && a == guardian;
    }

    function _requireText(string calldata name, string calldata ref, string calldata btcVault) private pure {
        if (bytes(name).length == 0 || bytes(ref).length == 0 || bytes(btcVault).length == 0) revert Empty();
    }

    function setPaused(uint256 id, bool paused_) external onlyOwner {
        _at(id).paused = paused_;
        emit Paused(id, paused_);
    }

    // ---- slow changes --------------------------------------------------------------------------------------------------------

    /// @notice The first roles apply at once; every later change waits CHANGE_DELAY in public.
    /// @dev docs/security/internal-review-v1.md M1. The deploy script rejected all three collisions, but it runs once, and a
    /// rotation is exactly when the check matters — rotation is what happens under the time pressure of a suspected key
    /// compromise. With `keeper != purchaser` a compromised keeper key alone can only send ETH to the purchaser's address and
    /// the attacker gets nothing; collapsed into one key it posts its own cap, declares against it and receives the ETH. And
    /// no role may be rotated onto an address some vault already treats as its autonomous buyer, for the same reason: that
    /// address can already pull ETH to itself, so it must never also be the key that bounds or pauses the pulling.
    function proposeRoles(address keeper_, address purchaser_, address attestor_) external onlyOwner {
        if (keeper_ == address(0) || purchaser_ == address(0) || attestor_ == address(0)) revert ZeroAddress();
        if (purchaser_ == attestor_ || keeper_ == attestor_ || keeper_ == purchaser_) revert SameKey();
        if (isAutoBuyer[keeper_] || isAutoBuyer[purchaser_] || isAutoBuyer[attestor_]) revert SameKey();
        // review2 N-8: the key that stops a role is never one
        if (_isGuardian(keeper_) || _isGuardian(purchaser_) || _isGuardian(attestor_)) revert SameKey();
        if (_keeper == address(0)) {
            _setRoles(keeper_, purchaser_, attestor_);
            return;
        }
        uint64 readyAt = uint64(block.timestamp + CHANGE_DELAY);
        pendingRoles = PendingRoles(keeper_, purchaser_, attestor_, readyAt);
        pendingRolesReadyBlock = _readyBlock();
        emit RolesProposed(keeper_, purchaser_, attestor_, readyAt);
    }

    function acceptRoles() external onlyOwner {
        PendingRoles memory p = pendingRoles;
        if (p.readyAt == 0) revert NothingPending();
        if (!Wait.elapsed(p.readyAt, pendingRolesReadyBlock)) revert TooEarly(p.readyAt);
        delete pendingRoles;
        _setRoles(p.keeper, p.purchaser, p.attestor);
    }

    function _setRoles(address k, address p, address a) private {
        _keeper = k; purchaser = p; attestor = a;
        emit RolesSet(k, p, a);
    }

    // ---- the keeper incident switch ------------------------------------------------------------------------------------------

    /// @notice The keeper every contract obeys: the named one, or zero while the keeper is paused. The floor module and the sweep
    /// vaults read it on every call, so a rotation (`proposeRoles`, seven public days) moves it everywhere at once.
    function keeper() public view returns (address) {
        return keeperPaused ? address(0) : _keeper;
    }

    /// @notice The keeper as named, paused or not (for pages and checks).
    function namedKeeper() external view returns (address) { return _keeper; }

    /// @notice The owner or the guardian, at once: no keeper power anywhere until the owner resumes it.
    function pauseKeeper() external {
        if (msg.sender != owner() && (msg.sender != guardian || guardian == address(0))) revert NotGuardian();
        keeperPaused = true;
        emit KeeperPaused(msg.sender, true);
    }

    /// @notice The owner only. A real resume (from paused) is stamped on both clocks (`keeperResumedAt`).
    function unpauseKeeper() external onlyOwner {
        if (keeperPaused) {
            keeperResumedAt = uint64(block.timestamp);
            keeperResumedAtBlock = uint64(block.number);
        }
        keeperPaused = false;
        emit KeeperPaused(msg.sender, false);
    }

    /// @notice The owner, at once: the guardian can only stop things (zero: none). It may be no other role, now or pending, and no
    /// auto-buyer (review2 N-8): the key that pauses the keeper must not be the keeper, and an owner key used as guardian would put
    /// the Safe's power on a hot host.
    function setGuardian(address guardian_) external onlyOwner {
        if (guardian_ != address(0) && guardian_ != guardian) {
            _requireNoRole(guardian_);
            if (isAutoBuyer[guardian_]) revert SameKey();
        }
        guardian = guardian_;
        emit GuardianSet(guardian_);
    }

    function proposeRoot(uint256 id, bytes32 root) external onlyOwner {
        if (_at(id).kind != Kind.Inscriptions) revert WrongKind();
        if (root == bytes32(0)) revert Empty();
        uint64 readyAt = uint64(block.timestamp + CHANGE_DELAY);
        pendingRoot[id] = PendingRoot(root, readyAt);
        pendingRootReadyBlock[id] = _readyBlock();
        emit RootProposed(id, root, readyAt);
    }

    function acceptRoot(uint256 id) external onlyOwner {
        PendingRoot memory p = pendingRoot[id];
        if (p.readyAt == 0) revert NothingPending();
        if (!Wait.elapsed(p.readyAt, pendingRootReadyBlock[id])) revert TooEarly(p.readyAt);
        delete pendingRoot[id];
        _listings[id].membershipRoot = p.root;
        emit RootSet(id, p.root);
    }

    function proposeBtcVault(uint256 id, string calldata btcVault) external onlyOwner {
        _at(id);
        if (bytes(btcVault).length == 0) revert Empty();
        uint64 readyAt = uint64(block.timestamp + CHANGE_DELAY);
        _pendingBtcVault[id] = PendingText(btcVault, readyAt);
        pendingBtcVaultReadyBlock[id] = _readyBlock();
        emit BtcVaultProposed(id, btcVault, readyAt);
    }

    function acceptBtcVault(uint256 id) external onlyOwner {
        PendingText memory p = _pendingBtcVault[id];
        if (p.readyAt == 0) revert NothingPending();
        if (!Wait.elapsed(p.readyAt, pendingBtcVaultReadyBlock[id])) revert TooEarly(p.readyAt);
        delete _pendingBtcVault[id];
        _listings[id].btcVault = p.value;
        emit BtcVaultSet(id, p.value);
    }

    /// @notice More protection applies at once; less waits seven public days, like every other loosening.
    function setBondBps(uint16 bps) external onlyOwner {
        if (bps > 10_000) revert BadBps();
        if (bps >= bondBps) {
            bondBps = bps;
            pendingBondAt = 0;
            emit BondBpsSet(bps);
        } else {
            pendingBondBps = bps;
            pendingBondAt = uint64(block.timestamp + CHANGE_DELAY);
            pendingBondReadyBlock = _readyBlock();
            emit BondBpsProposed(bps, pendingBondAt);
        }
    }

    function acceptBondBps() external onlyOwner {
        if (pendingBondAt == 0) revert NothingPending();
        if (!Wait.elapsed(pendingBondAt, pendingBondReadyBlock)) revert TooEarly(pendingBondAt);
        bondBps = pendingBondBps;
        pendingBondAt = 0;
        emit BondBpsSet(bondBps);
    }

    function proposeTreasury(address treasury_) external onlyOwner {
        if (treasury_ == address(0)) revert ZeroAddress();
        if (isAutoBuyer[treasury_] || _isGuardian(treasury_)) revert SameKey();
        pendingTreasury = treasury_;
        pendingTreasuryAt = uint64(block.timestamp + CHANGE_DELAY);
        pendingTreasuryReadyBlock = _readyBlock();
        emit TreasuryProposed(treasury_, pendingTreasuryAt);
    }

    function acceptTreasury() external onlyOwner {
        if (pendingTreasuryAt == 0) revert NothingPending();
        if (!Wait.elapsed(pendingTreasuryAt, pendingTreasuryReadyBlock)) revert TooEarly(pendingTreasuryAt);
        treasury = pendingTreasury;
        pendingTreasury = address(0);
        pendingTreasuryAt = 0;
        emit TreasurySet(treasury);
    }

    /// @dev The parent-chain block at which a CHANGE_DELAY proposed now has passed.
    function _readyBlock() private view returns (uint64) {
        return uint64(block.number + Wait.blocksFor(CHANGE_DELAY));
    }

    // ---- views ---------------------------------------------------------------------------------------------------------------

    function count() external view returns (uint256) { return _listings.length; }
    function listing(uint256 id) external view returns (Listing memory) { return _at(id); }
    function vaultOf(uint256 id) external view returns (address) { return _at(id).vault; }
    function isPaused(uint256 id) external view returns (bool) { return _at(id).paused; }
    function pendingBtcVault(uint256 id) external view returns (string memory value, uint64 readyAt) {
        PendingText storage p = _pendingBtcVault[id];
        return (p.value, p.readyAt);
    }

    /// @notice The leaf for an inscription id. Hashed twice so that an inner node can never be passed off as a leaf.
    function leafOf(string memory inscriptionId) public pure returns (bytes32) {
        return keccak256(bytes.concat(keccak256(bytes(inscriptionId))));
    }

    function isMember(uint256 id, string calldata inscriptionId, bytes32[] calldata proof) external view returns (bool) {
        Listing storage l = _at(id);
        if (l.kind != Kind.Inscriptions) return false;
        return MerkleProof.verifyCalldata(proof, l.membershipRoot, leafOf(inscriptionId));
    }

    function _at(uint256 id) private view returns (Listing storage) {
        if (id >= _listings.length) revert NoListing();
        return _listings[id];
    }
}
