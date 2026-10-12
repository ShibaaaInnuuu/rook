# The buyer on other EVM chains: buying a listed NFT through Seaport

Written 2026-09-23, **before its code**. Applies to `autobuyer/canister-rs`. Phase one of the EVM-NFT feature: the buyer
canister buys one ERC-721 from one Seaport listing on a chain other than Robinhood Chain (4663), with the threshold ECDSA key
it already has. The vault, raffle and burn side on 4663 is a separate piece of work (the ListingRegistry, agent B); this
document says what it needs from that registry (§9).

Why: Nosh sells "external collections" where "buying happens off-chain by the keeper". Here the keeper only **hands in** an
order. The canister checks it against the chain and against a cap recorded before the order arrived. Then it builds its own
calldata, signs it and broadcasts it, and it believes only the read-back. The keeper cannot change what is bought, what it
costs or where the NFT goes.

The design is **generic**: nothing in the logic is specific to Ethereum. A chain is one entry in a configuration list (§4).
The only address the code knows is Seaport 1.6's, and that is not tied to one chain. Seaport 1.6 is deployed by deterministic
CREATE2 at `0x0000000000000068F116a894984e2DB1123eB395` on every chain surveyed below.

---

## 1. Where NFT activity is, and what each chain offers the canister (measured 2026-09-23, read-only)

A canister needs four things from a chain:

- **Seaport 1.6** deployed there.
- **At least two keyless JSON-RPC providers** that the IC can reach over IPv6 (IC HTTPS outcalls are IPv6; IPv4 goes through
  a SOCKS proxy, which works but is a second path to trust). The state they serve must go deep enough to answer at a pin
  (docs/v2/pinned-reads.md).
- **A Relay route** from 4663, to bring the ETH over.
- **Listings it can fill** without an API key.

Providers were probed with `eth_getBalance` at the head minus 1,024, 8,192 and 65,536 blocks ("state depth", y = answered).
IPv6 means the host has an AAAA record. Block times were measured over about 1,000 blocks.

| chain (id) | block time | Seaport 1.6 code | keyless providers with deep state **and** IPv6 | shallow or IPv4-only | Relay 4663 → |
|---|---|---|---|---|---|
| Ethereum (1) | 12.10 s | 23,981 B, `information()` = "1.6" | `eth.drpc.org`, `xrpc.cl/eth` | tenderly (IPv4), publicnode (shallow), cloudflare-eth (head 0, broken) | yes |
| Base (8453) | ~2 s | 23,981 B, "1.6" | `mainnet.base.org`, `developer-access-mainnet.base.org`, `xrpc.cl/base`, `base.drpc.org` | tenderly, baseazul (IPv4), publicnode (shallow) | yes |
| ApeChain (33139) | 0.68 s | 23,981 B | `rpc.apechain.com`, `apechain.drpc.org`, `apechain.calderachain.xyz/http` | — | yes |
| Abstract (2741) | 0.44 s | present (zkEVM bytecode) | `abstract.drpc.org` only (and it is shallow at 1,024) | `api.mainnet.abs.xyz` (IPv4 only) | yes |
| Arbitrum One (42161) | 0.26 s | 23,981 B | `arb1.arbitrum.io/rpc`, `xrpc.cl/arbitrum` | publicnode, arbitrum.drpc.org (shallow) | yes |
| Polygon PoS (137) | 1.50 s | 23,981 B | `polygon.drpc.org`, `xrpc.cl/polygon` | quiknode, tenderly (IPv4), publicnode (shallow) | yes |
| Blast (81457) | 2.00 s | 23,981 B | `blast.drpc.org` only | `rpc.blast.io` (IPv4), publicnode (shallow) | yes |

The Seaport runtime code is the same length everywhere, but **its keccak differs on every chain**, because Seaport keeps its
chain id and EIP-712 domain separator as immutables:

| chain | Seaport code hash |
|---|---|
| 1 | `0x74499ac0cce14428e4b41541d5e44f28f5a6882a1051d0118867c2a93cd5aec0` |
| 8453 | `0x2d5cb8553e21a19550413299c05a6493cc606fb0000b5042328a5ba06bfdaf35` |
| 33139 | `0x1bbbe3ba82d3f18986d3b1a4830b887b406b81ccef0f46cd35677ddcacdb9463` |
| 42161 | `0x297200fa59c084fe1b9991faf59fcf018c3598a415aab8dd30ba61246261f703` |
| 137 | `0x24fd52b9fba5545dabf736449aa4f2beed9562a4a641d2f6c85ca05eeed35d52` |
| 81457 | `0x1e8391434a35d0ce280e81940906f0a030f68f2ff1d4af56858a2808b87b8624` |
| 2741 | `0xc00dc1e5382340b4c57057dbd0d63038ad6ad8aae26c521357316f719e33d50d` |

So a chain's configuration carries its own code hash (§4), and the canister also checks `information()`. There, the domain
separator must equal the one the canister computes from `(chain id, Seaport address)`. That check is what makes the canister's
own order hashes (§6) mean the same thing Seaport means.

**Where the volume is.** Public sources (CoinGecko's NFT and chain pages, The Block's NFT overview, Electric Capital's NFT
dashboard, cryptonomist.ch 2026-03-25) agree that Ethereum dominates by dollar volume: about $720M a month for ETH NFTs in Q1
2026. Polygon and Solana lead by count. None of them gave a trustworthy per-chain figure for Base, ApeChain or Abstract.

What we measured ourselves, from Seaport's own `OrderFulfilled` events:

- **Ethereum, last 3,000 blocks (about 10 h):** 2,851 fills. Of these, 1,921 were a single ERC-721 sold for native ETH, and
  **1,899 of those (99%) were open orders (zone 0)**, which a keyless buyer can fill (§5). One sweep-heavy collection accounts
  for a large share.
- **Base, last 3,000 blocks (about 100 min):** 76 fills. 13 were ERC-721 for ETH, 5 of them open.
- **ApeChain:** no `OrderValidated` events in 400,000 blocks. Fills were not sampled.

**Recommendation: Ethereum and Base first. ApeChain third, if listing volume there is shown.**

- **Ethereum** is where the collections worth raffling are. It has two IPv6 providers from different operators that serve deep
  state, and open listings are the norm there.
- **Base** is cheap to rehearse on and has four IPv6 providers, including Coinbase's own. Real, open, keyless-fillable listings
  trade on it every few minutes. It proves the path is generic.
- **ApeChain** has three IPv6 providers, including the chain's own, but we found no evidence of volume.
- **Abstract** and **Blast** each have only one IPv6 provider with deep state, so they need the SOCKS path. They are not ready.
- **Arbitrum** and **Polygon** qualify on infrastructure. They are second-tier for NFT dollar volume.

## 2. What is signed and where the NFT goes

The canister calls `fulfillBasicOrder` (selector `0xfb0f3ee1`) on Seaport. The order is a *basic order* for native ETH in
exchange for one ERC-721 (`basicOrderType` 0 or 1: ETH → ERC-721, full or partial *open*).

**Custody: the NFT comes to the canister's own EVM address, not straight to a raffle winner.** A basic order delivers to
`msg.sender`, so no field in the order can send the NFT anywhere else. That is the strongest recipient guarantee Seaport
offers, and it needs no recipient check at all. It also fits the timing: the purchase happens before the draw on 4663 resolves.
The winner is paid from custody by a later `transferFrom`, which is phase two and B's registry's concern.
`fulfillAdvancedOrder`, which takes a recipient, would allow direct delivery, but it would also bring in criteria resolvers,
partial fills and a recipient field that the verifier would have to police. It is not used.

The canister's EVM address is the same on every chain (one threshold key, derivation path `evm`). Custody on chain X is that
address on chain X.

## 3. Pinned reads, per chain

docs/v2/pinned-reads.md applies unchanged to every chain, with **one per-chain parameter: the bucket**. `pin::BUCKET = 1,024`
is tuned to 4663's 0.104 s blocks, where it is about 106 s. On Ethereum, 1,024 blocks would be 3.4 hours: the pin would be
3.4–6.8 hours old, far too stale to tell whether a listing is still live or the fee is still sane. The bucket is therefore part of each chain's configuration.

The rule: **the bucket is a power of two covering about 90–130 s of blocks.** Nodes' requests a second or two apart then rarely
straddle a boundary. The pin sits 1 to 2 buckets plus the provider lag behind the head, which is within every listed
provider's state window.

| chain | bucket | pin depth |
|---|---|---|
| Ethereum | 8 | 97–194 s |
| Base | 64 | 128–256 s |
| ApeChain | 128 | 87–174 s |
| Abstract | 256 | 113–226 s |
| Arbitrum | 512 | 133–266 s |
| Polygon | 64 | 96–192 s |
| Blast | 64 | 128–256 s |

Everything else is as it is on 4663:

- heads are rounded in a transform
- `pin = min(rounded) − bucket`
- every provider must agree on the pinned block's hash
- every state read names the pin by hash (EIP-1898)
- a receipt counts only at or below the pin
- a gas estimate is the highest over providers at the pin's number

The pinned block's **timestamp** is added to what is agreed. It is the chain's own clock, and an order's `endTime` is judged
against it. The canister's clock is not used: Seaport checks `block.timestamp`, not IC time.

## 4. Configuration: a list of chains

The install argument gains one optional field, so an old argument or an old saved state still decodes:

```
evm_chains : opt vec record {
  chain_id : nat64;            // also checked against eth_chainId on every use
  rpc_urls : vec text;         // ≥ 2, distinct
  bucket : nat64;              // §3
  seaport : text;              // must be the canonical 1.6 address
  seaport_codehash : text;     // keccak of Seaport's runtime code on THIS chain (§1), checked at the pin
  max_fee_wei : nat64;         // hard cap on maxFeePerGas; a pin whose base fee needs more is refused
  priority_fee_wei : nat64;    // maxPriorityFeePerGas
  gas_cap : nat64;             // hard cap on the gas limit
  fee_recipients : vec text;   // marketplace fee recipients accepted in an order (e.g. OpenSea's), per chain
  max_fee_bps : nat32;         // everything paid to anyone but the offerer, at most this share of the total
  min_order_life_s : nat64;    // an order must stay valid at least this long past the pin's timestamp
}
```

Validation, at install and at every upgrade:

- chain ids are distinct and non-zero
- **the home chain (`chain_id` of the vault) may not be listed**. The same key has the same address there, and its nonce line
  belongs to the vault loop; two writers on one nonce line break "one transaction in flight"
- at least two distinct URLs
- `bucket ≥ 1`
- the Seaport address is the canonical one
- the code hash is 32 bytes
- the fee cap is above zero
- `max_fee_bps ≤ 10,000`

An upgrade takes `evm_chains` from its argument, as it does `rpc_urls`, so a provider change is a published timelock proposal.
An upgrade that drops a chain with a purchase in flight is refused.

## 5. Where listings come from, with no API key

The canister never holds or asks for a marketplace API key. Two sources exist:

1. **The relayer hands in the order.** The keeper takes it from wherever it likes: OpenSea's API with its own key, a
   marketplace page, or a fill it watched. It hands the canister the `BasicOrderParameters` ABI encoding, meaning the
   `fulfillBasicOrder` calldata (either selector, `0xfb0f3ee1` or the gas-efficient alias `0x00000000`; trailing bytes such as
   OpenSea's 4-byte tracking tag are ignored). **The canister never sends the keeper's bytes.** It decodes them, verifies them
   (§6), and re-encodes its own calldata from the decoded fields. The fulfiller conduit key is zeroed and nothing is appended.
2. **On-chain validated orders** (`Seaport.validate`, event `OrderValidated`) are public and need no signature. They are rare:
   7 on Ethereum in 60,000 blocks, none on Base in 300,000, none on ApeChain in 400,000. That makes them a supplement, not a
   source.

**Restricted orders are refused in phase one.** Most OpenSea listings on Base, and a minority on Ethereum, use OpenSea's
SignedZone (`0x000056F7000000EcE9003ca63978907a00FFD100`). Seaport calls that zone to authorise the fill, and the zone wants an
`extraData` signature that only OpenSea's API (with a key) hands out. A basic order passes empty `extraData`, so such a fill
reverts. Filling restricted orders would mean `fulfillAdvancedOrder` plus a zone signature relayed by the keeper (§10).

## 6. The verifier: what the canister refuses

Pure code (`seaport.rs`), run on every preview and again on every sign. It refuses anything but:

1. **The canonical Seaport.** The transaction's `to` is the configured Seaport, which must be the canonical address. At the pin,
   its code hash must equal the configured one, and `information()` must return version "1.6" with the domain separator the
   canister computes for this chain.
2. **ETH for one ERC-721, on an open order.** `basicOrderType` is 0 or 1. The consideration token and identifier are zero.
3. **Exactly the listed token.** The offer token is the target collection, the offer identifier is the target token id, and the
   offer amount is 1.
4. **To us.** A basic order delivers to `msg.sender`, and the canister signs the transaction from its own address. Nothing in
   the order can name another recipient. The offerer is neither us nor zero.
5. **Total ≤ the recorded cap.** Total = `considerationAmount` + Σ `additionalRecipients[].amount`, computed with overflow
   checks. The cap was recorded for (chain, collection, token id) **before** the order was handed in, and the order never sets
   it.
6. **No money to strangers.** Every additional recipient is either one of the chain's configured marketplace fee recipients or
   the collection's ERC-2981 royalty receiver, read at the pin. None is the zero address. There are no tips:
   `totalOriginalAdditionalRecipients` equals the list's length. All of them together receive at most `max_fee_bps` of the total.
7. **Not expired.** `startTime ≤ pin timestamp` and `endTime > pin timestamp + min_order_life_s`.
8. **Still fillable at the pin.**
   - `ownerOf(tokenId)` is the offerer.
   - `getOrderStatus(orderHash)` shows the order is not cancelled and not filled. The order hash is computed by the canister
     (EIP-712 `OrderComponents` with the offerer's `getCounter` at the pin).
   - An `eth_call` of the exact transaction (from us, with the value, to Seaport) succeeds at the pin. This is what checks the
     signature, the offerer's approval of its conduit, and any contract-wallet signer.
9. **Signed by the offerer**, where that can be checked without the chain: for a 64- or 65-byte signature, EIP-712 recovery must
   give the offerer. Longer signatures (bulk orders, ERC-1271) are left to the simulation in rule 8.

Fees: `maxFeePerGas = base fee at the pin × 4 + priority`. It is refused above the chain's `max_fee_wei`. The gas limit is the
highest estimate × 1.2, refused above `gas_cap`. The balance at the pin must cover `total + gas × maxFee`.

## 7. The nonce line: one transaction in flight per chain

Each chain has its own line in stable memory. It holds at most one purchase that is signed but not yet settled:

1. **Sign.** Refused if the line holds an unsettled purchase. The nonce is `eth_getTransactionCount(us)` **at the pin**. The
   canister verifies (§6), builds, signs, and **saves the raw bytes and their hash before anything is sent**.
2. **Send.** Sends the saved bytes to every provider, each from one node. Every reply is a report, not a fact. Sending again is
   safe: one nonce means at most one transaction is mined.
3. **Status.** The replicated read-back: the receipt must be at or below the pin, with every provider agreeing. When it is
   mined and succeeded, `ownerOf(tokenId)` at a pin at or above its block must be us. Only then is the purchase recorded as
   **arrived** and the line cleared. If it reverted, it is recorded as reverted and the line is cleared; the ETH spent on gas is
   gone, and the reason is logged. If the pinned nonce has moved past ours with no receipt for our hash, some other transaction
   took the nonce. That is logged loudly and the line is cleared, because nothing else signs for this address on this chain.

## 8. Getting the ETH there: Relay, quote only

Relay routes 4663 → every chain in §1. Real quotes captured on 2026-09-23 for 0.1 ETH:

- 4663 → 8453: 0.09979 ETH out, about 2 s
- 4663 → 1: 0.09977 ETH out, about 5 s

The deposit is the same `depositNative(depositor, id)` to the same depository `0x4cd0…bc31` on 4663 that
`relayquote.rs` already checks for the BTC route.

The canister **asks for the quote and judges it; it never transacts**. The rules are those of `relayquote.rs`:

- one deposit step on 4663, to the known depository, for exactly the amount
- calldata that names us as depositor
- the destination chain is the one asked for
- the currency out is native (the zero address)
- the stated recipient is **our own EVM address**
- the amount out is at least the floor

As with the BTC bridge, the only proof of arrival is **reading the destination chain**: our balance at a pin on chain X, with
every provider agreeing. Signing the deposit is left for when the user accepts Relay's residual trust. It is the same decision
as for the BTC route and is not made here.

## 9. What B's ListingRegistry needs to carry, per listing

- `chainId` (uint64): the chain the NFT is bought on. It must be one of the canister's `evm_chains`.
- `collection` (address): the ERC-721 contract on that chain.
- `tokenId` (uint256): or none, for "any token of the collection". Phase one buys a **named** token only, because the verifier
  matches the exact id.
- `capWei` (uint256), or a way to derive it: the price ceiling, recorded before any order exists. The canister takes the cap
  from the registry (or the keeper's recorded cap), never from the order.
- **Custody:** the NFT is held at the canister's EVM address on `chainId` until the draw, then transferred to the winner. The
  registry does not need a per-chain recipient. It does need a way to name the winner's address **on `chainId`**, which is the
  same 20 bytes as on 4663 unless the winner is a contract.

## 10. Proof plan and known limits

- Unit tests on **real orders** captured read-only:
  - an Ethereum fill, tx `0xc3d3dd48…` in block 26,037,608 (ERC-721 `0x9763…3043` #9820, 0.053 ETH)
  - a Base fill, tx `0x61ba2cd1…` in block 51,672,614 (ERC-721 `0x15f3…b6a6` #8901, 0.00486 ETH)
  - an Ethereum validated restricted order, block 26,027,018

  The canister's order hash must equal the `OrderFulfilled`/`OrderValidated` hash, recovery must give the offerer, and every
  rule in §6 must refuse its tampered case.
- **Fork proofs** with anvil on port 8608, each fork taken at the block **before** the real fill, where the listing was still
  live. The canister (a fresh local canister, local replica key) previews, signs and broadcasts. The NFT must arrive at the
  canister's address, and a tampered copy of the order must be refused before signing. This is done once on Ethereum and once
  on Base, with the same code and a different chain entry. The fork keeps the real chain id, so the real order signature is
  valid there. The signed bytes are sent only to anvil. The local key's address holds nothing on either real chain.
- Limits, stated:
  1. Restricted (SignedZone) orders are out of scope (§5).
  2. The keeper chooses *which* order to hand in. It cannot overpay past the cap, but it can pick a worse listing under the
     cap. Choosing the cheapest listing needs a listing source, and none is keyless.
  3. On the fork, both "providers" are one anvil reached by two URLs. The agreement check runs, but it cannot disagree.
  4. The Relay bridge is judged, not signed.

---

## 11. Results (2026-09-23), recorded after the code

**The same code and two chain entries bought a real listing on two chains.** The canister was a fresh one on the local
replica (`jdh4q-l7777-77777-aaaoa-cai`, key `key_1`). Its EVM address, `0xcb787bae732a520c3736c61fc950b967f8ee6ed2`, has a
balance of 0 and a nonce of 0 on the real Ethereum and Base. Each fork was an anvil taken at the block before the real fill.
Run it with `autobuyer/scripts/nft-fork-rehearsal.sh`.

| | Ethereum fork (chain 1, :8608, bucket 8) | Base fork (chain 8453, :8609, bucket 64) |
|---|---|---|
| listing | `0x9763…3043` #9820 from `0x09e3…2deb`, 0.053 ETH incl. a 0.00053 OpenSea fee | `0x15f3…b6a6` #8901 from `0x4455…8159`, 0.0048638 ETH incl. fee |
| order hash (canister) | `0xb3ba1356…a4fd4e`, equal to Seaport's `OrderFulfilled` | `0xae391908…9ce0bb`, equal |
| signature | recovered to the offerer in the canister | recovered to the offerer |
| tampered, refused before signing | stranger fee recipient: *unknown recipient*; price −1 wei (under the cap): *signature*; price +1 ETH: *over cap*; token id +1: *wrong token* | the same four, same refusals |
| second sign while in flight | *refused (in flight)* | — |
| signed and sent | `0xa0528454…51d973`, nonce 0; the providers reported Accepted / NonceTooLow (the same anvil twice) | `0x6aede6e0…67591d`, nonce 0 |
| the node's own recovery | anvil's receipt: `from` = our address, status 1 | the same |
| `nft_status` | `Arrived` (block 26,037,644, pin 26,037,656) | `Arrived` (block 51,672,762, pin 51,672,768) |
| `ownerOf` afterwards (read by cast, not the canister) | our address | our address |

What this proves:

- the checks, the order hash, signature recovery and our own calldata work on real listings on two chains
- the pinned per-chain reads (bucket 8 and 64), code hash and `information()` checks, and the simulation all passed
- EIP-1559 signing with the threshold key produced a sender that the node itself recovered to our address
- the send, the read-back and arrival by `ownerOf` all worked
- the same run also upgraded the canister with state (the second `install` was an upgrade) and kept its lines

What it does not prove:

- **Provider disagreement.** Both "providers" were one anvil, so the check could not disagree.
- **Mainnet cycles and timing.** A preview took about 60 s of local outcalls.
- **The Relay leg.** It is judged in tests only.
- **Restricted orders.**
- **The Base clock.** On the Base fork, the fork's clock was set back to just after the fork block, because the real listing
  expired 401 s after it. The canister judged expiry by the pinned block's timestamp, as designed.

Verification at commit `ff7e99b`:

- `cargo test`: 214 passed (178 lib, 2 e2e, 20 loop, 3 policy_loop, 11 redeem), 0 failed
- wasm: 628,390 B gzipped, up from 579,194 at `f24e234`
- `forge test` in `autobuyer/`: 15 passed; at the root: 298 passed. No Solidity was changed.

Hooks outside the new modules (`canister.rs`):

- `Init.evm_chains`
- `validate` → `chains::validate`
- `restore_init` takes `evm_chains` from the upgrade argument
- `pre_upgrade`/`post_upgrade` save and restore a sixth optional element. An upgrade that drops a chain with a purchase in
  flight is refused.
- `rpc_once_with`, `send_raw_once`, `hex_bytes`, `hex_u128` and `h0x` are now `pub(crate)`

`looper.rs` is untouched: the NFT line is separate from the vault loop's.

---

## 12. Restricted OpenSea listings: SignedZone orders via `fulfillAdvancedOrder` (2026-09-23, written before its code)

§5 refused restricted orders, and §10's first limit named what that costs. On Base, 8 of 13 recent ERC-721-for-ETH listing
fills were OpenSea SignedZone orders. This section lets the canister fill them. **It adds one new untrusted input, the zone's
`extraData`, and one new party with a key, the keeper's OpenSea API key.** The canister never sees that key.

### 12.1 What was measured (read-only)

- **The zone.** It is `0x000056F7000000EcE9003ca63978907a00FFD100` on every chain, and `getSeaportMetadata()` returns
  "OpenSeaSignedZone".
  - SIP-5 schema 7 (SIP-7). Its substandards are **[1, 7, 8, 9]**. Its EIP-712 domain separator on Base is `0x70bc8510…556b`.
  - The runtime code is 6,967 bytes. Its keccak differs per chain: Base `0x9cc021be99f4db02490d261e6fe6c3792bd2e777d10ff799150a2eaecf41c701`,
    Ethereum `0x9e6930be6759e905574791278e46db4d510c072733fe62a0c7d4e6e76f13e324`. It is not a proxy: the EIP-1967 slot is empty.
  - The only signer that `getActiveSigners()` returns on Base is `0x2F16…8Fd2`. The signer set is a `mapping(address => bool)` at
    slot 0. `updateSigner(address,bool)` reverts `InvalidController` for anyone except the immutable controller
    `0x0000…657e498d54000053b69bdaad008a001dbf`.
- **A real fill** (Base tx `0xc812f2c3…`, block 51,673,875). It is `fulfillAdvancedOrder` (`0xe7acab24`):
  - `orderType` 2 (full restricted), the zone above, numerator = denominator = 1, and no criteria resolvers.
  - The offerer's signature is a **bulk-order** signature: 64 bytes, then a 3-byte index and a 1-level proof.
  - `extraData` is 146 bytes: SIP-6 version `0x00` | expected fulfiller (20) | expiration (uint64) | EIP-2098 signature (64) |
    context. The context is substandard 8: `0x08` ‖ identifier (32) ‖ registry (20). The registry is `0x721c008f…42e3`, Limit
    Break's creator-token transfer validator.
  - The zone signature recovers under `SignedOrder(address fulfiller,uint64 expiration,bytes32 orderHash,bytes context)`, with
    the zone's domain, **to `0x2F16…8Fd2`, the active signer**. So we have read the format correctly, and a real node agrees.
- **Zone signatures are short-lived.** Across three real fills, `expiration` was **276 s, 281 s and 288 s after the block that
  mined them**, so about 5 minutes from issue. This sets the budget: the keeper fetches, then the canister checks, signs and
  sends, all within about 5 minutes.

### 12.2 The route (keeper side), from OpenSea's current docs

`POST https://api.opensea.io/api/v2/listings/fulfillment_data`, header `x-api-key`
(https://docs.opensea.io/reference/generate_listing_fulfillment_data_v2; since the fulfillment-endpoints changelog it is
"the only sanctioned way to retrieve fulfillment data").

The request body:

```
{ "listing": { "hash": <order hash>, "chain": "base", "protocol_address": "0x0000000000000068f116a894984e2db1123eb395" },
  "fulfiller": { "address": <the canister's EVM address> } }
```

The response has `fulfillment_data.transaction`, which carries `function`, `to`, `value`, `chain` and `input_data`. For this kind
of order, `input_data` is `{ advancedOrder: { parameters, numerator, denominator, signature, extraData }, criteriaResolvers,
fulfillerConduitKey, recipient }`. On a 429, OpenSea's docs say to back off until `Retry-After` (seconds) or
`X-RateLimit-Reset`.

The keeper:

- reads `OPENSEA_API_KEY` from `process.env` when it makes the call, never at load time, and never logs it; the logger redacts
  the value wherever it appears
- retries through `backoff.ts`
- checks that `function` is `fulfillAdvancedOrder` or `fulfillBasicOrder`, that `to` is Seaport 1.6 and that `chain` is the one
  it asked for
- ABI-encodes `input_data` into calldata, and hands the canister **exactly what it hands it today**: `nft_preview` /
  `nft_sign(chain_id, collection, token_id, order_hex)`

Neither the key nor any header ever reaches the canister. The canister's interface is unchanged.

### 12.3 What the canister accepts (`seaport.rs`, `nftnet.rs`)

The canister accepts `fulfillAdvancedOrder` only when **all** of the following hold. Everything in §6 still holds as well:
collection and token id, the cap, fee recipients or ERC-2981, validity at the pin, `getOrderStatus`, `ownerOf`, the offerer's
signature, and the exact `eth_call` at the pin.

1. The zone is on this chain's `signed_zones` allowlist, a new optional config field (address + code hash). At the pin, the
   zone's code hash equals the configured one. `orderType` is 2 or 3 (restricted).
2. Numerator = denominator = 1, no criteria resolvers, one offer item (ERC-721, amount 1), and every consideration item native
   ETH with `startAmount == endAmount`. The first consideration item is paid to the offerer.
   `totalOriginalConsiderationItems` equals the number of items: no tips.
3. **The canister builds its own calldata** from the decoded order: recipient is ours, fulfiller conduit key is zero,
   criteria resolvers are empty, and nothing is appended. The relayer's recipient and conduit key are discarded.
4. **`extraData` is parsed and refused unless all of these hold:**
   - SIP-6 version is `0x00`
   - the expected fulfiller is **our own address**
   - the expiration is later than `max(pinned block timestamp, IC time) + min_order_life_s`
   - the signature is 64 bytes
   - the context is empty, or a sequence of known substandards (1: identifier; 7: identifier, registry, operator; 8/9:
     identifier, registry), where each identifier equals the first consideration item's identifier and each registry is on this
     chain's `zone_registries` allowlist (optional)

   Anything else is refused as substandard.
5. **The offerer's signature** is recovered here for plain EOA signatures **and for bulk orders**. Bulk orders are rebuilt from
   Seaport's `BulkOrder` tree typehash and the proof. Anything else is left to the simulation.

The canister does not check the zone signer's signature itself. It cannot know the signer set without trusting a read of the
zone's storage, and the simulation checks it anyway.

### 12.4 Why `extraData` cannot move the NFT or raise the price

- **What is paid, and to whom**, comes only from `parameters.consideration`. That is hashed into the order hash, which the
  offerer signed, and the canister has checked every item of it against the recorded target, the cap and the fee allowlist.
  The canister also sets `msg.value` itself, equal to the checked total.
- **Where the NFT goes** is `recipient` in our calldata, set to our own address. It is not a field of `extraData` and not a field
  the zone sees. Seaport transfers the offer items to the recipient before it asks the zone anything.
- **What the zone can do** with `extraData` is return the magic value or revert, from `authorizeOrder` and `validateOrder`. Both
  calls are made by Seaport with `ZoneParameters` built by Seaport. The zone's code is pinned by address and code hash to
  OpenSea's deployed contract, which has no path to move tokens or ETH. Substandards 7, 8 and 9 make the zone call a transfer
  validator (the `registry`), which can in turn only approve or revert; the registry is allowlisted too.
- **Could it make us overpay?** No: the price is fixed before `extraData` is looked at. At most, a bad `extraData` turns a fill
  into a revert. The simulation at the pin catches a revert before signing. A revert after signing, for example an expiry in
  the gap between signing and mining, costs gas and nothing else. The purchase settles as `Reverted` and the line clears.

### 12.5 Evidence plan

- **Unit tests** on the real Base fill `0xc812f2c3…`:
  - the order hash equals the one in `OrderFulfilled`
  - the bulk signature recovers to the offerer
  - the zone signature recovers to the active signer
  - every `extraData` tamper is refused: expired, another fulfiller, an unknown substandard, a registry not on the allowlist,
    a wrong version, a wrong length
  - the keeper's calldata for the same order decodes to the same fields
- **Fork** (anvil at :8629, Base forked at 51,673,874, the block before the real fill), using OpenSea's real zone and the real
  bulk-signed listing:
  - **(a)** the real `extraData`, which names another fulfiller, is refused by the canister before signing
  - **(b)** a zone signature for the canister's address cannot be obtained without the key; OpenSea issues them only to the
    fulfiller it names. So on the fork **only**, a test signer (anvil's public dev key #1) is added to the real zone through
    its real controller path: `anvil_impersonateAccount(controller)`, then `updateSigner`. Nothing is written to the zone's
    code or storage directly. With that signer's `extraData` for our address, the canister checks, signs, sends, reads back,
    and the NFT arrives
  - **(c)** an expired signature, and one from a signer the zone does not know, are refused before signing

  What (b) substitutes is exactly one thing: who signed the zone's approval. That is stated wherever it is reported.
- **A live check** of the OpenSea route needs the user's key. The exact command is given in §12.7 once built, for the user to
  run. It is never run here.

### 12.6 Timing limit, stated

The real window is about 5 minutes. The canister's pinned reads on Base run 128–256 s behind the head, and on the IC a sign
is about 20 replicated outcalls. The keeper must therefore call `nft_sign` (which checks everything again), **not**
`nft_preview` then `nft_sign`, and then `nft_send` at once. A listing younger than the pin cannot be bought: at the pin it
does not exist yet. For restricted orders on Base, a smaller bucket (32 blocks, about 64 s) is a per-chain setting worth
measuring.

### 12.7 Results (2026-09-23), recorded after the code

**Fork run** (`autobuyer/scripts/nft-zone-fork-rehearsal.sh`):

- Setup: anvil on :8629, Base forked at 51,673,569. The canister is a fresh local one, `jkexm-5x777-77777-aaapq-cai`, with EVM
  address `0xaccc…7427`. On real Base that address holds 0 with nonce 0.
- Listing: the real SignedZone listing of `0x266d…3041` #6895 for 0.00032 ETH, filled for real one block later by
  `0x8f1d…f7c7` in tx `0xb22b9f9e…`. Its bulk signature recovers to the offerer inside the canister.
- The order reached the canister through the keeper's `toHandoff`, fed OpenSea's documented response shape with the real
  values. OpenSea's answer was mocked; OpenSea was not called.

| step | result |
|---|---|
| an outsider calls the zone's `updateSigner` | reverts `InvalidController` |
| the real controller (impersonated on the fork) adds anvil's public dev key #1 | `isActiveSigner` = true |
| **(a)** the real order with its real `extraData` | refused before signing: *the zone signed for fulfiller 0x8f1d…f7c7, not us* |
| **(c)** a zone signature for us expiring within 60 s | refused before signing: *expires … within 60 s of now* |
| **(c)** a zone signature for us from a key the zone does not know | refused before signing by the simulation. The real zone reverted `SignerNotActive(address,bytes32)` at the pin |
| **(c)** an unknown substandard (5) | refused before signing: *substandard 5 is not one this canister accepts* |
| **(b)** the real order + the test signer's `extraData` for us, via the keeper | checked, signed and sent: tx `0xae979d3f…`. `nft_status` returned **Arrived** (block 51,673,719, pin 51,673,728). anvil's receipt shows `from` = our address and status 1, with logs from the collection and Seaport. `ownerOf(6895)`, read by cast, is our address |

**The only substitution is the zone signer** (§12.5). OpenSea's real `extraData` for this order names another buyer, and the
canister refuses it, as it must. Everything else on the path to Arrived was real: the order, its bulk signature, Seaport 1.6,
the zone's code and its signer check (which rejected the unknown key), the transfer validator
`0xa000027a…0000` its substandard 8 calls, and the chain state.

**A real refusal, kept as evidence.** The first SignedZone fill found, `0xc812f2c3…` (`0xc7c1…bd33` #4711), pays 10% to
`0x3918…855b`. That collection's `royaltyInfo` returns the zero address, so this creator fee is OpenSea's off-chain setting,
and nothing on chain vouches for its recipient. Rule 6 refuses it, both in the unit test and on a fork at 51,673,874. Buying
such listings would need the recipient recorded **with the target, before the order**, for example as a field in B's registry.
That is not done here (open problem).

**Two transfer validators appear in real `extraData`:** `0x721c008f…42e3` and `0xa000027a…0000`. Both are on the fork
config's `zone_registries`. Production config must list what it accepts, per chain.

**Verification at this commit:**

- `cargo test`: 219 passed, 0 failed. The new tests are the SignedZone decoder and verifier on two real fills, the order-hash,
  bulk-signature and zone-signer recoveries, 25 `extraData`, shape and §6 tampers, and old-config decoding.
- wasm: 644,182 B gzipped, up from 628,390.
- keeper: `npm test` gives 35 passed. `tsc` is clean.
- `forge test`: 489 passed at the root and 15 in `autobuyer/`.

**The live check, for the user to run.** It is never run here. `railway run` injects `OPENSEA_API_KEY` into this one process
without printing it, and the output is only the canister arguments. Pick a current Base SignedZone listing, take its order hash
from OpenSea, and use the canister's EVM address:

```
cd keeper && railway run -s keeper -- npx tsx -e 'import { fulfillmentData, candidArgs } from "./src/opensea.ts"; import { logger } from "./src/log.ts";
  fulfillmentData({ chainId: 8453, orderHash: "<ORDER_HASH>", fulfiller: "<CANISTER_EVM_ADDRESS>" }, { log: logger() })
    .then((h) => console.log(h.zoneExpiration, candidArgs(h)), (e) => console.error(String(e.message)))'
```

It passes if it prints an expiration about 5 minutes ahead and a `fulfillAdvancedOrder` order hex whose `extraData` names
`<CANISTER_EVM_ADDRESS>`. That result settles the one unverified assumption: that OpenSea issues a zone signature to a fulfiller
address that has never been authenticated. SIP-7 lets a signer require EIP-4361 sign-in for a named fulfiller, and a canister
cannot do an EIP-4361 sign-in through the keeper without signing a message. If OpenSea answers `FulfillerRequired`, the design
needs that sign-in. **This is the largest open question.**

## Live check (2026-09-23): OpenSea issues zone approvals to the canister's address

Run with the operator's OpenSea key (read-only; no transaction): for a real restricted Base listing (collection `basedones`,
order `0x6a09a12d…b051f3`, zone `0x000056f7…ffd100`, orderType 2, 0.0005 ETH), `POST /api/v2/listings/fulfillment_data` with
`fulfiller = 0xcb787bae732a520c3736c61fc950b967f8ee6ed2` (the local canister's address) answered `fulfillAdvancedOrder` to
Seaport 1.6 with `extraData` present and the canister named as the fulfiller/recipient. No sign-in (EIP-4361) was demanded.
Open problem 1 of §12 ("whether OpenSea will issue a zone signature to the canister's address") is closed. Note for the keeper:
OpenSea's API sits behind Cloudflare, which answers 403 (error 1010) to requests without a `User-Agent` header, and the
collections endpoint's `order_by=seven_day_volume` returned 500 that day — the check found collections from Seaport
`OrderFulfilled` logs on Base instead (23 ERC-721 collections in 6,000 blocks).
