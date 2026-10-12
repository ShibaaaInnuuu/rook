# The NFT wrapper: one ERC-20 unit per blue-chip NFT held by the canister on its own chain

Written 2026-09-23, **before its code**, on branch `nft-wrapper` (from `6eb96e6`). This builds docs/v4/nft.md §8, which was
written down and not built. Its concern there, stated plainly: an NFTX-style "redeem any id" wrapper lets the first redeemers take
the best ids, so the token trades at the worst id in the pile. The answer here is the one `src/Wrapper.sol` already gives for
Bitcoin inscriptions (docs/v3/wrapper.md): **you cannot choose the id**. A whole token burns to a ticket pinned to a drand round
that does not exist yet, and the round picks which NFT leaves.

Nothing here is deployed or audited.

## In one paragraph

`NftWrapper` is an ERC-20 on Robinhood Chain (4663), 18 decimals, one per collection (e.g. `wAPE` for Bored Apes on Ethereum,
chain 1). `1e18` units are one NFT of `(chainId, collection)` held at the canister's own EVM address on that chain. Only the
canister (`custodian`, immutable, the same 20 bytes on 4663 and on the collection's chain) mints, one `1e18` per NFT that entered
custody, recording the token id and the transaction that brought it in. A holder burns `1e18` for a random NFT from the held set,
delivered to their own address on the collection's chain (or a different address they name at redeem); the canister records the
delivery by transaction hash; a ticket nobody delivers within 7 days (both clocks) is refunded by anyone, the NFT back in the set
and `1e18` back to the redeemer. Burn listings redeem to `0x…dEaD` on the collection's chain the same way. There is no owner, no
pause, no setter and no withdraw.

## 1. Why it is Wrapper, re-keyed, and not a new mechanism

`Wrapper.sol` already solved every hard part of a one-unit-per-item custodial wrapper with random redemption, and its security
tests (test/security/WrapperSecurity.t.sol) pin down attacks that are not obvious:

| Wrapper's decision | why it exists | here |
|---|---|---|
| a burn pins `beacon.roundAt(now) + 20`, a future drand round | nobody knows the draw when burning | **same** |
| tickets resolve strictly in order (`nextToResolve`) | resolving a later ticket first would change what an earlier one draws from | **same** |
| every item that enters the set is stamped with `tickets.length`; ticket `n` promotes only items stamped `≤ n` | a mint (or refund) after the round is public would otherwise move which item a known random number lands on | **same** |
| index = `keccak(randomness, wrapper, ticketId) % freeSize`; swap-and-pop | uniform, depends on nothing a caller supplies | **same** |
| `redraw` after 24 h on both clocks if drand skipped the round | liveness without letting a caller choose between rounds | **same** |
| `refund` after `DELIVERY_DEADLINE` (7 d) on both clocks, from `Resolved` only; item back to the queue, `1e18` to the owner | a custodian that stops cannot destroy tokens for nothing | **same** |
| `Wait.elapsed(time, parentBlock)` for every wait | the 4663 sequencer can stamp an hour ahead | **same** |
| items identified by acquisition id; `inscriptionHeld[leaf]` blocks a second mint while held | one item, one unit | items are numbered `itemId`; `itemOf[tokenId]` blocks a second mint while held |

What changes is only what is Bitcoin-shaped in Wrapper:

- **Minting.** Wrapper mints on a receipted, `Never`-ruled acquisition from an AutoBuyer vault, or on a holder's Bitcoin deposit.
  Here the canister mints directly on its own attestation that the NFT is at its address at a finalized block (§5): the canister
  buys through Seaport into its own address (docs/v4/eth-nft-canister.md §2), so there is no 4663 vault in between to receipt
  from. That is the same trust `NftRemoteVault.receipt` already places in the canister ("a false receipt ... this chain cannot
  read the other one"), and the same trust Wrapper places in its custodian for `wrapDeposit`.
- **The destination** is a 20-byte EVM address on the collection's chain, not a taproot string.
- **Deliveries are recorded by the custodian only.** Wrapper also lets the keeper record (Bitcoin deliveries predate the
  custodian having its own EVM voice). Here the canister signs the delivering transaction from the custody address itself; nobody
  else knows its hash first, so nobody else needs the power to record it.
- **No registry, no distributor, no listing.** The wrapper names its collection by `(chainId, collection)`, both immutable. It does
  not pause: pausing would strand redeemers' tickets, and minting is already one key's decision.

## 2. The contract: `src/nft/NftWrapper.sol`

```solidity
constructor(IRandomnessBeacon beacon, uint64 chainId, address collection, address custodian, string name, string symbol)
```

Refused: a zero beacon, collection or custodian; `chainId == 0`; `chainId == block.chainid` (a 4663 collection belongs in
`NftSweepVault`'s contract custody, and the canister's home chain is refused as an `evm_chains` entry anyway); a custodian with
code (the canister's address is a threshold-ECDSA EOA, the same check `NftRegistry.listNft` makes for a buyer).

### 2.1 Minting — the custodian only, one unit per NFT in custody

```solidity
function mint(uint256 tokenId, bytes32 purchaseTx, address to) external returns (uint256 itemId);        // bought via Seaport
function mintDeposit(uint256 tokenId, bytes32 depositTx, address to) external returns (uint256 itemId);  // a holder's deposit, §4
```

Both: `msg.sender == custodian`; `to != 0`; the tx hash is non-zero; the token id is **not currently held** (`itemOf[tokenId] ==
0`); and `(tokenId, txHash)` has **never** minted (`sourceUsed`), so replaying an old attestation after the NFT has left and come
back cannot mint twice. Then: a new item `{tokenId, sourceTx, deposit}` is appended, `itemOf[tokenId] = itemId + 1`, `held++`,
the item is queued with stamp `tickets.length`, and `ONE_ITEM` is minted to `to`.

`to` is whoever the canister is filling: the curve during graduation, the mint desk / per-collection buy queue for a paid request,
the depositor for a deposit. That is how Wrapper's `wrapDeposit(…, to)` is used by FrogCurve and MintDesk today.

A token id may enter custody again after it left (delivered, then bought back later) with a **new** transaction hash: that is a
new item, and the old ticket still names the old item. An id sent to `0x…dEaD` never comes back.

### 2.2 Redeeming — nobody chooses the id

```solidity
function redeem(address destination) external returns (uint256 ticketId);   // burn 1e18; deliver to `destination` on chainId
function redeemToBurn() external returns (uint256 ticketId);                 // burn 1e18; deliver to 0x…dEaD on chainId
function redraw(uint256 ticketId) external;                                  // anyone, if drand skipped the round (24 h, both clocks)
function resolve(uint256 ticketId, bytes signature) external;                // anyone, strictly in order
function recordDelivery(uint256 ticketId, bytes32 txHash) external;          // the custodian only
function refund(uint256 ticketId) external;                                  // anyone, 7 days after resolution, both clocks
```

- `redeem(destination)`: `destination` is the redeemer's choice, fixed for ever at the burn. The UI passes the redeemer's own
  address (the "same 20 bytes" default); a redeemer whose 4663 account is a contract with no twin on the collection's chain names
  another. Refused: zero (there is no implicit default, so a contract redeeming for someone cannot silently send to its own
  address on a chain where it has no code), and `custodian` (a "delivery" to custody moves nothing and would take the item off the
  books while it is still there). `0x…dEaD` is allowed and is exactly `redeemToBurn`.
- **No function takes a token id or an item id from a redeemer**, and the exact-ABI test holds that. The only inputs to the draw
  are the ticket id (assigned), the drand randomness (unknown at burn) and the free set (fixed by stamps before the round exists).
- `resolve`, `redraw`, `refund`: Wrapper's code, re-keyed from acquisition ids to item ids.
- `recordDelivery`: from `Resolved` (also past the deadline while nobody has refunded, as in Wrapper). Clears `itemOf[tokenId]`,
  `held--`, `openTickets--`. A `0x…dEaD` delivery also counts `burned++`.

### 2.3 The ledger

`totalSupply + openTickets × 1e18 == held × 1e18` at every step, equivalently `totalSupply == (held − openTickets) × 1e18`:
each held NFT backs one whole token or one open ticket, never both. `held == number of token ids t with itemOf[t] != 0`: no token id
is counted twice while held. Every held item is in exactly one place: the free set, the pending queue, or named by one `Resolved`
ticket. These are the invariant suite's assertions (§7).

### 2.4 What this does NOT protect against, because nothing on 4663 can

- **A lying or compromised custodian.** It can mint against an NFT it does not hold, or move a held NFT without recording it.
  Supply and backing then diverge and this contract cannot see it. The custodian is the whole question; this contract keeps the
  books honest. Anyone can reconcile: for every held item, `ownerOf(tokenId)` on `chainId` must be `custodian`. The monitor should
  do exactly that daily (§5.4).
- **A collection contract that can move tokens it does not own** (an admin transfer, an upgradeable proxy). Choose collections
  whose `transferFrom` is plain ERC-721. The wrapper cannot see the other chain.
- **The delivery-and-refund race.** A delivery sent near `deliverBy` AND a refund on 4663 leaves the redeemer with both. The
  canister stops sending a day before `deliverBy` and re-reads the ticket immediately before signing (§5.3), as for Bitcoin.
- **A beacon that never proves a round**: that ticket and every ticket behind it wait. Refund is from `Resolved` only.
- **A collection whose items differ wildly in value.** Random redemption means a token is worth about the *average* item held,
  not the floor and not the best. Rare items should not be minted into a floor wrapper; the canister buys floor listings under a
  cap (docs/v4/eth-nft-canister.md §6), and a depositor of a grail gets `1e18` like anyone. That is the depositor's choice (§4).

## 3. The shared shape: `src/interfaces/IWrappedCollection.sol`

The curve (`FrogCurve`), the mint desk, `SweepVault` and the per-collection buy queue use the wrapper through a handful of calls
(read on `6eb96e6`):

| consumer | what it calls on `Wrapper` |
|---|---|
| `FrogCurve` | `custodian()`, `balanceOf`, `allowance`, `approve` / `transfer` (via SafeERC20) |
| `MintDesk` | `custodian()`, `balanceOf`, `transfer` |
| `SweepVault` | `balanceOf`, `transfer`, `transferFrom`, `redeem(BURN_ADDRESS)`, `ticket(id).stage == Refunded`, and at construction `registry()` / `listingId()` |
| `CurveFactory` | holds the wrapper and passes it on |

Everything except the burn and the refund check is ERC-20 plus `custodian()`. The burn is Bitcoin-shaped in Wrapper
(`redeem(string)` with a taproot constant held by the *vault*); the refund check reads a Bitcoin-shaped struct. So the interface
moves the burn destination into the wrapper (the wrapper knows what "provably unspendable" means on its chain) and reduces the
refund check to a boolean:

```solidity
interface IWrappedCollection is IERC20 {
    /// 1e18: one whole token is one item in custody.
    function ONE_ITEM() external view returns (uint256);
    /// The canister: the only minter, and the only address a curve, desk or queue should accept "I minted it here" from.
    function custodian() external view returns (address);
    /// Items in custody and counted: free, pending, or named by a resolved, undelivered ticket.
    function held() external view returns (uint256);
    /// Burns neither delivered nor refunded. totalSupply == (held − openTickets) × ONE_ITEM.
    function openTickets() external view returns (uint256);
    /// Burn ONE_ITEM of the caller's for a random item delivered where nobody can ever move it again. Returns the ticket.
    function redeemToBurn() external returns (uint256 ticketId);
    /// Whether `ticketId` was refunded (its ONE_ITEM minted back to its owner).
    function refunded(uint256 ticketId) external view returns (bool);
}
```

**`NftWrapper` implements it** (`contract NftWrapper is ERC20, IWrappedCollection`, so the compiler checks it).

**`Wrapper` implements four of the six today** (`ONE_ITEM`, `custodian`, `held`, `openTickets`, plus the ERC-20). It lacks two
one-liners, which its owner adds on the generic-curve branch (this branch does not touch `src/Wrapper.sol`):

```solidity
string public constant BURN_ADDRESS = "bc1p2zffkaxp5py4fdutfdsrt6t6tcrc5ks09rkfd428hlhf4n5q8tqqwv7ahk";  // SweepVault's NUMS output
function redeemToBurn() external returns (uint256) { return _redeem(BURN_ADDRESS); }   // redeem's body, as an internal taking memory
function refunded(uint256 id) external view returns (bool) { return _tickets[id].stage == Stage.Refunded; }
```

With those, `SweepVault.retire` becomes `wrapper.redeemToBurn()` and `reclaim` becomes `wrapper.refunded(ticketId)`, for either
wrapper. `SweepVault`'s constructor check `wrapper.registry() == registry && wrapper.listingId() != listingId` is Bitcoin-listing
specific and stays outside the interface; a generic vault should check `custodian()` / code instead.

Deliberately **not** in the interface: `redeem(destination)` (its destination type differs per chain — a taproot string or an
EVM address — and no contract consumer redeems to a person), `ticket(id)` (the struct differs), minting (custodian-only and
chain-shaped), and any identity getter (`chainId`, `collection`, `registry`): consumers that need to know *which* collection read
the concrete type.

## 4. Holder deposits: a holder's NFT in, one unit out, recipient bound by the holder's own signature

**Built on the contract side (`mintDeposit`), specified for the canister.** The recipient binding is safe under one restriction,
stated below; a deposit that does not meet it is not minted.

**How the depositor names the 4663 recipient: in the transfer transaction's calldata, signed by the owner.** The holder sends,
from the NFT's owner EOA, a **top-level** transaction to the collection:

```
collection.safeTransferFrom(owner, custodian, tokenId, data)      // selector 0xb88d4fde
data = "EWN1" ‖ uint32 BE 4663 ‖ wrapper address (20 bytes) ‖ recipient on 4663 (20 bytes)      // exactly 48 bytes
```

The `custodian` has no code, so the collection makes no `onERC721Received` call and `data` goes nowhere on chain; it exists only
in the transaction's input, which the owner signed. That is the whole binding: **the recipient is chosen by whoever signed the
transaction that moved the NFT, and nobody else can change it after signing.** There is no 4663-side claim, so there is nothing
to front-run. This is the EVM equivalent of Wrapper's Bitcoin `OP_RETURN` (docs/v3/sweep.md, 1a).

**The restriction: a top-level call from the owner.** `data` is visible to a keyless RPC only as `tx.input` of a top-level call
(`eth_getTransactionByHash`); inside a contract call (a Safe, a 4337 smart account, a marketplace) it is visible only in a trace,
which the canister's providers do not serve. So the canister requires `tx.to == collection`, `tx.input` decodes as
`safeTransferFrom(address,address,uint256,bytes)`, and the decoded `from == tx.from`. An approved operator (a marketplace conduit
holding `setApprovalForAll`) moving the NFT does **not** qualify: `tx.from` must be the owner. A smart-account holder deposits by
first moving the NFT to an EOA they control. EIP-7702-delegated EOAs are fine (the top-level call is still theirs).

**The canister's checks (spec, not built).** A deposit `(chainId, txHash)` is attested only if every one holds, all read at a
pinned **finalized** block on `chainId` (§5.1) with every provider agreeing:

1. The receipt exists at or below the pin, `status == 1`, and the block is at or below the chain's `finalized` tag (Ethereum:
   the beacon chain's finalized checkpoint, ~13 min; Base: the L2 block whose batch is finalized on L1, ~20–30 min). A reorg past
   finality is out of scope; one short of it is excluded by construction.
2. `tx.to == collection`, `tx.from == from`, input decodes as above, `to == custodian`, `data` is exactly 48 bytes with magic
   `EWN1`, chain `4663`, wrapper `== this wrapper`, recipient non-zero.
3. The receipt contains the collection's `Transfer(from, custodian, tokenId)` log (the call really moved that id).
4. `ownerOf(tokenId) == custodian` at the pin (it is still there, not moved on by a later transaction).
5. `(tokenId, txHash)` is not already minted and `itemOf(tokenId) == 0` on 4663 (the contract checks both again).
6. A daily deposit cap (e.g. 20 a day per collection), so a stolen key's quiet minting is bounded in rate.

A deposit that fails 2 (no data, wrong format, sent through a contract) is **not minted**. The NFT sits at the custodian's address
unaccounted for. The canister returns it to `tx.from` by `transferFrom` on the same nonce line, as policy (not code), and logs it.
It is never silently wrapped to someone.

**The contract side (built): `mintDeposit(tokenId, depositTx, to)`**, custodian only, with the same checks as `mint` and a
separate event, so the audit trail says which items came from purchases and which from holders.

**Attack surface of deposits, stated:**

- *Front-running*: none (no claim exists).
- *A third party naming themselves*: they would need to sign a transaction from the owner's key.
- *Double mint*: a token id held is refused; a `(tokenId, txHash)` pair mints once, ever.
- *Reorg*: bounded by finality (check 1).
- *A grail deposited to a floor wrapper*: the depositor's loss and the next random redeemer's gain. Not a vulnerability.
- *A lying canister*: same key, same trust as custody itself; the deposit path widens what a stolen key can do quietly (mint
  rather than only steal), hence the rate cap and the daily reconciliation.

## 5. The canister side (spec only; `canister.rs` is being edited on another branch)

Everything below fits in **new modules** (`nftwrap.rs` for the logic, reusing `nftnet.rs` for pinned reads and `evmsend.rs` /
`evmtx.rs` for signing) plus a small wiring hook; it is not built on this branch.

### 5.1 Pinned, finalized reads

Purchases in docs/v4/eth-nft-canister.md are judged at a pin 1–2 buckets behind the head. **Minting needs more**: a mint on 4663
is irreversible, so the NFT's arrival must be final. The mint pin is the chain's `finalized` block (`eth_getBlockByNumber
("finalized")`), rounded down to the chain's bucket, with every provider agreeing on its hash; all reads name it by hash
(EIP-1898). `ownerOf(tokenId)` at that pin must be `custodian`.

### 5.2 Mint attestation (the purchase path)

After `nft_status` reports `Arrived` for a purchase (docs/v4/eth-nft-canister.md §7, 3) and the purchase's block is at or below the
finalized pin: `ownerOf(tokenId) == custodian` at the finalized pin, then call `NftWrapper.mint(tokenId, purchaseTxHash, to)` on
4663 through the vault loop's 4663 nonce line (the canister's home chain), where `to` is the request being filled (curve, desk or
queue). The canister stores `(chainId, tokenId, purchaseTx) → minted` before sending, so a restart re-sends the same call rather
than a new one; the contract refuses a second `(tokenId, tx)` anyway.

### 5.3 Delivery (phase two: transfer to a winner)

For each ticket in `Resolved` stage, in ticket order:

1. Read the ticket on 4663: `itemId`, `destination`, `deliverBy`. Refuse if `now > deliverBy − 1 day` (stop sending a day before
   the deadline; the redeemer's refund is the remedy).
2. Read `item(itemId).tokenId`; `ownerOf(tokenId) == custodian` at a fresh pin on `chainId`.
3. Build `collection.transferFrom(custodian, destination, tokenId)` — **never `safeTransferFrom`**: a destination contract that
   reverts in `onERC721Received` could otherwise block its own delivery, collect the refund after 7 days, and redeem again for
   another draw — a re-roll. With plain `transferFrom`, a recipient cannot refuse.
4. **Immediately before signing**, re-read the ticket stage on 4663: still `Resolved` (not refunded). Sign on `chainId`'s nonce
   line (docs/v4/eth-nft-canister.md §7), which holds one transaction in flight per chain and is shared with purchases.
5. After the receipt is final and `ownerOf(tokenId) == destination`, call `recordDelivery(ticketId, txHash)` on 4663.

A burn ticket is the same with `destination = 0x…dEaD`. The canister refuses `0x…dEaD` as a custody input (nobody can sign for it).

### 5.4 Reconciliation (monitor, not canister)

Daily: for every item with `itemOf(tokenId) != 0`, `ownerOf(tokenId) == custodian` on `chainId`; `held` equals that count;
`totalSupply + openTickets × 1e18 == held × 1e18`. Any mismatch is an alarm.

## 6. Constants

| constant | value | why |
|---|---|---|
| `ONE_ITEM` | `1e18` | as Wrapper |
| `PIN_AHEAD` | 20 drand rounds | as Wrapper (a minute of rounds) |
| `REDRAW_AFTER` | 24 h, both clocks | as Wrapper |
| `DELIVERY_DEADLINE` | 7 days, both clocks | as Wrapper; an EVM delivery takes minutes, the rest is margin for an outage |
| `DEAD` | `0x000000000000000000000000000000000000dEaD` | as `NftVaultBase.DEAD` |
| `DEPOSIT_MAGIC` | `"EWN1"` | the deposit data prefix (§4), exposed so UIs build the same bytes |

## 7. Tests (`test/nftwrap/`)

- **Unit** (`NftWrapper.t.sol`): constructor refusals; minting (custodian only, zero inputs, held id refused, `(id, tx)` refused
  for ever, re-entry after delivery with a new tx); deposits; redeem (destination rules, burn); resolve order and draw; delivery
  (custodian only, stage, burn count); refund (both clocks, item back to the set); redraw; events.
- **Fuzz**: arbitrary mint/redeem/resolve sequences keep the ledger; a drawn item is always held and never drawn twice; the draw
  is the same whoever resolves; a refunded ticket's item is drawable again.
- **Invariants** (`NftWrapper.invariant.t.sol`): the ledger; every held item in exactly one place; `itemOf` consistent with the
  books and no token id held twice; tickets resolve in order; no required transition panics or sticks; a second mint of a held id
  never succeeds; refund/delivery totals reconcile.
- **Security** (`NftWrapperSecurity.t.sol`, mirroring WrapperSecurity): minting after the round is public cannot move the draw
  (N1); refund timing cannot (N2); resolution order is not a choice (N3); the stamp is real (N4); who resolves cannot (N5); a
  redeemer cannot choose or change anything after the burn (N6); the sequencer clock cannot open a refund or a redraw early
  (S1, S2); exact ABI; no way out by name; the interface is implemented.
- **Real drand** (`NftWrapperDrand.t.sol`): a burn pins the round a real evmnet beacon signed (test/fixtures), a wrong signature
  is refused, and the real signature resolves to the formula's NFT.

## 8. Built: what the code settled (2026-09-23, with the code)

- `src/nft/NftWrapper.sol` and `src/interfaces/IWrappedCollection.sol` are new. **No existing file was edited**: not
  `Wrapper.sol`, the curve, the desk, `SweepVault`, `Wait.sol`, `ListingRegistry.sol` or `NftSweepVault.sol`.
- Items are numbered (`itemId`), not keyed by token id, so a token id that leaves and comes back is a second item with its own
  source transaction, and every ticket keeps naming the item it drew. `itemOf[tokenId]` stores `itemId + 1` so zero means "not
  held".
- `sourceUsed[keccak256(abi.encode(tokenId, txHash))]` is shared by `mint` and `mintDeposit`: one attestation mints once, on
  either path.
- `Stage` has Wrapper's values in Wrapper's order.
- `burned` counts deliveries to `DEAD`, whichever function made the ticket.
- Verification: `forge build` clean (lint notes only, of the kinds the existing contracts carry); `forge test --no-match-path
  "test/fork/*"`: **527 passed, 0 failed**, of which **52 are in `test/nftwrap/`** (35 unit and fuzz, 14 security, 1 real-drand,
  and the invariant suite: 8 invariants over 64 runs × 64 calls with 0 reverts, plus a by-hand driver of every transition). No
  fork test was needed: the wrapper reads nothing on 4663 but its beacon, and nothing was sent to any chain.

## 9. Open problems

1. **`Wrapper` does not yet implement `IWrappedCollection`.** It needs `redeemToBurn()` and `refunded(uint256)` (§3). Until the
   generic-curve branch adds them, a consumer typed on the interface cannot take a `Wrapper`.
2. **Mint attestations are the canister's word** (§2.4), as receipts are in `NftRemoteVault`. Proving `ownerOf` at a finalized
   block of another chain on 4663 needs a light client or a storage-proof oracle; neither exists here.
3. **Value spread inside a collection.** Random redemption prices the token at the average held item. A floor wrapper stays a
   floor wrapper only if the canister buys floor and depositors do not deposit grails; the second is their choice.
4. **Holder deposits exclude smart accounts** (§4): the binding is readable only from a top-level transaction. A Safe or 4337
   holder must move the NFT to an EOA first. Accepting internal calls needs `debug_trace*`, which keyless providers do not serve.
5. **The canister side is spec only** (§5): finalized-pin reads, mint attestation, deposit verification, delivery by
   `transferFrom` on the per-chain nonce line and the daily reconciliation. Another branch is editing `canister.rs`; the natural
   home is a new `nftwrap.rs` with a hook in the EVM chain loop.
6. **A collection whose transfers can be refused** (a transfer allow-list, a recipient block list) could make a delivery
   impossible; the ticket then refunds after 7 days and the redeemer may try again, which is a re-roll for that redeemer only.
   Choose collections with plain ERC-721 transfers; the canister should refuse a destination such a collection would reject.
7. **The deadline race** (§2.4) is the canister's rule, as for Bitcoin, not code.
