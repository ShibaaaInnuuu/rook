# The venue, the bridge and the purchase policy

Written 2026-09-23, before its code. The buyer's loop (`docs/v2/buyer-loop.md`) runs end to end only against fixtures for three
things: the **venue** (where the rune is bought: a quote, then a PSBT to sign), the **bridge** (ETH on Robinhood Chain 4663 →
BTC at the buyer's own taproot address) and the **purchase policy** (nothing decides when to open a purchase). This document says
what exists for each, what the canister can verify, and what is built.

**The constraint that shapes all three.** Any API key placed in the canister is public: install arguments are readable, and the
timelock publishes every upgrade's argument in full for 72 hours (`docs/v2/timelock-controller.md`). A venue or bridge that
needs a key cannot be called from the canister with it. IC HTTPS outcalls go over IPv6 first; for an IPv4-only host, DFINITY's
docs say *"The IC first tries a direct IPv6 connection; if that fails (e.g., the server is IPv4-only), it retries through a
SOCKS proxy"* (docs.internetcomputer.org/concepts/https-outcalls, read 2026-09-23 — the page does not say who runs the proxy or
whether every subnet has it). **Correction to this document's first version,** which called IPv4-only hosts unreachable.

## 1. The policy — built: an on-chain trigger, no operator discretion

**What on chain means "there is ETH owed to a purchase".** An `AutoBuyerVault` (src/AutoBuyerVault.sol) credits each coin's
fees to it; once a coin's unspent fees reach `MIN_ENTRY` (0.01 ETH) the coin is queued and its wei count in `attributed` —
spendable, and owed to that coin's purpose, which is the listing's asset. `announce(maxEth, quantity, capRef)` succeeds only
when: the listing is not paused; `block.timestamp ≤ capExpiry` (the keeper's cap, fresh for an hour); `maxEth ≤
boundFor(quantity)`; nothing overdue blocks the vault (`isBlocked`); fewer than `MAX_OPEN` (3) purchases are open; `maxEth ≤
maxInFlight − inFlight`; `maxEth ≤ attributed`; and the coins it charges number at most `MAX_PAYERS` (64).

**The trigger, frozen.** At the start of every `tick`, when no purchase is unfinished (one at a time, as today) and a venue and
a bridge are configured, the canister reads a **snapshot** — replicated, every provider agreeing — of: the listing
(`registry.listing(listingId)`: kind, paused, divisibility, `ref` = the rune id, `btcVault` = where the rune must land),
`attributed`, `capWei`, `capExpiry`, `maxInFlight`, `inFlight`, `openPurchases().length` and `isBlocked()`; "now" is the
canister's own clock (a sequencer running ahead only makes `announce` revert, which the loop reads as "wait").
`policy::candidates`, `first_fundable` and `order_for` then open a purchase **iff** all hold:

1. the listing is a **rune** (the loop settles runes only: `settleRune`) and is **not paused**;
2. the vault is **not blocked** and has **no open purchase** (the buyer is the only announcer, so an open one is ours or stuck);
3. the cap is fresh for at least `CAP_MARGIN_SECS` = **15 minutes** more (announce lands a few blocks later, and must still
   find the cap unexpired);
4. `amount = min(attributed, maxInFlight − inFlight, MAX_PURCHASE_WEI)` is at least `MIN_PURCHASE_WEI` = **0.05 ETH**, and the
   route's fixed costs (an estimate the rates module supplies; a fixture until it exists) are at most **5%** of it;
5. `payersFor(amount)` is between 1 and 64 — otherwise the amount is halved until it is, or the policy waits.

What it opens: `amount_wei = amount`; `units = ceil(amount × 10^divisibility / capWei)` — the fewest smallest units the vault's
bound allows for that ETH, so the buyer never promises more than the cap price buys; `rune = listing.ref`; `recipient_script`
= `listing.btcVault` decoded (bech32/bech32m, mainnet or the installed network's HRP) to its scriptPubKey; `cap_ref =
"cap:<capWei>@<capExpiry>"`. Every input is on chain; nothing is the operator's word.

`open_purchase` (the operator's manual door) is **refused on chain 4663**: there, only the policy opens. It stays for rehearsal
installs, where the policy's snapshot comes from a local chain.

`MIN_PURCHASE_WEI`, `MAX_PURCHASE_WEI` (**0.5 ETH**), `CAP_MARGIN_SECS` and the 5% fee ratio are chosen here, not fitted, and are
constants in `src/policy.rs`; changing them is a code change, so it passes the timelock.

## 2. The bridge — no single verifiable hop exists; a two-hop route does, and is designed, not executable yet

Robinhood Chain's own bridging page (docs.robinhood.com/chain/bridging, read 2026-09-23) names the Arbitrum canonical bridge
(~7-day withdrawal), LayerZero/Stargate, Chainlink CCIP, Relay, Across and LiFi/0x. **Nothing native to Bitcoin.**

**Relay — reachable in principle, unverifiable in fact.** Relay's keyless API (api.relay.link/chains, read 2026-09-23) lists
4663 and Bitcoin (id 8253038). A real quote for 0.1 ETH → BTC returned 315,473 sats, ~$2 in fees, ~210 s. But the transaction
the canister would sign is `depositNative(address depositor, bytes32 id)` to `0x4cd00e38…bc31`: **the Bitcoin destination is
not in the calldata**; it is bound to `id` in Relay's database. A relayer that fetched the quote with its own BTC address hands
the canister calldata indistinguishable from an honest one. `bridgecall::verify` therefore refuses it as a relayer-supplied call.
Relay is examined in full in §2a, because it is the only live one-hop route.

**The verifiable route: Across, then Chainflip.** Each hop names its destination in its own calldata, so the canister decodes
and checks it before signing:

- **Hop 1, Across: 4663 ETH → Arbitrum ETH, to the buyer's own EVM address.** Keyless API with AAAA (app.across.to). SpokePool
  on 4663 `0xD29C85F15DF544bA632C9E25829fd29d767d7978` (from `/api/suggested-fees`), wrapped native `0x0Bd7D308…AD73` (read from
  the pool). `depositV3(depositor, recipient, inputToken, outputToken, inputAmount, outputAmount, destinationChainId,
  exclusiveRelayer, quoteTimestamp, fillDeadline, exclusivityDeadline, message)` is on the deployed implementation
  (`0x1771c470…edd8`, selector `0x7b939232` present). A real quote, 2026-09-23: 0.01 ETH → 0.009992747 WETH on Arbitrum, 0.0725%,
  ~1 s fill. **But liquidity is tiny:** the same route refused 0.1 ETH ("Max amount is 0.0623 WETH"), then 0.05 ETH ("0.0222"),
  within a minute. At today's depth the route carries dust, not a purchase.
- **Hop 2, Chainflip: Arbitrum ETH → native BTC, to the buyer's taproot address.** Vault on Arbitrum
  `0x79001a5e762f3bEFC8e5871b42F6734e00498920` (docs.chainflip.io mainnet addresses, read 2026-09-23).
  `xSwapNative(uint32 dstChain, bytes dstAddress, uint32 dstToken, bytes cfParameters)`; Bitcoin is chain **3**, BTC asset **5**
  (Chainflip SDK `enums.ts`); a Bitcoin `dstAddress` is the **UTF-8 bytes of the address string** (SDK `executeSwap.ts`).
  `cfParameters` carries refund parameters and broker data. The current docs build it through a broker's
  `broker_request_swap_parameter_encoding` RPC; an older SDK passed `'0x'`. **Open: whether a broker is required today**, and
  if so, whose — a broker is a registered account, and its RPC is not public.

### 2a. Relay, examined (the coordinator's lead, 2026-09-23)

> **Superseded 2026-09-26 (`docs/v6/relay-conversion.md` §0.2).** Relay now publishes the full order (`protocol.v2.orderData`),
> and the deposit's `id` is its EIP-712 `hashStruct`: recomputing it binds the deposit to the recipient, the minimum out and the
> refund addresses. Point 2 below ("nothing can prove where the BTC goes before signing") no longer holds; `relayconv::judge`
> replaces `relayquote::judge`, and no outcall to Relay is needed.

1. **The calldata.** 68 bytes: `depositNative(address depositor, bytes32 id)` — `cast calldata-decode` of the real quote's
   deposit gives depositor `0x5b0b…ebc2` (us) and id `0xa6e892a9…7475`, which is **not** the `requestId` Relay reports
   (`0x17901316…6425`). No recipient, no amount out.
2. **Can the canister verify id → recipient before signing? No.** Before a deposit, `GET /requests?id=` (either id) returns
   `{"requests":[]}`, `GET /intents/status?requestId=` returns `{"status":"unknown"}`, and `/intents/status/v2` returns
   `{"status":"waiting","quoteCreatedAt":…}` — no recipient anywhere. `GET /requests/v3` demands `x-api-key`; the keyless
   `/requests/v2` is deprecated ("throttledFrom 2026-09-01", "sunsetAt" 2026-11-24, from its own response). And `POST /quote`
   mints a new id on every call, so a **replicated** quote read cannot reach consensus even through a transform. After a
   deposit, the request's recipient can be read — which detects a substitution, too late to prevent it.
3. **Reachability.** `api.relay.link` is CloudFront with **no AAAA record**; per DFINITY's docs above, the IC retries IPv4-only
   hosts through a SOCKS proxy, so a (non-replicated) quote call from the canister should reach it. Not shown from an IC node.
4. **The depository `0x4cd00e387622c35bddb9b4c962c136462338bc31` on 4663:** 8,628 bytes of code, **not a proxy** (both
   EIP-1967 slots zero), `depositNative` and `depositErc20` present; `owner()` = `0xF61A…775A`, `allocator()` = `0x63C1…1b56` —
   deposits are released by Relay's allocator, so the ETH is Relay-custodied between deposit and fill. What happens to a
   deposit Relay cannot fill (refund to depositor?) is **not stated in anything read here — open**.
5. **Testnet.** `api.testnets.relay.link/chains` lists only Sepolia and Base Sepolia: **no route from 46630, none to
   signet/testnet4.** No rehearsal is possible; this work stops at quotes.
6. **Terms and limits.** docs.relay.link (API keys page): `/quote` 50 requests a minute, others 100–200; a key raises limits and
   is required for `/requests/v3` and for quotes that set a `referrer`. relay.link/terms answered 403 to a fetch and was not read.
   Recipient addresses are screened (the coordinator saw `BLOCKED_WALLET_ADDRESS` for a known address).

**The smallest safe way to use Relay, and what it still trusts.** The canister asks Relay for the quote **itself**
(non-replicated outcall), naming its own taproot address as recipient — so no relayer or operator can substitute a
recipient — and signs only if `relayquote::judge` passes: exactly one deposit step, on 4663, to the known depository, for exactly
the amount, calldata naming us as depositor, the quote's stated recipient our own address, native BTC out, at least our floor.
Still trusted: **Relay** (that `id` really is bound to what its quote said, and that it fills) and **the one replica** that served
the quote (it could substitute another quote's `id`). Bounded by the per-purchase ceiling (`MAX_PURCHASE_WEI`, 0.5 ETH) and
`maxInFlight`; arrival is still proven only by reading Bitcoin, and a bridge that never pays ends `Stuck(Bridge)` and is written
off in public. **Built as a pure, tested judge; not configured** — accepting that residual trust is the user's decision.

**What is built:** `src/relayquote.rs` — the Relay judge above, tested on the real captured quote
(`tests/fixtures/relay-quote-4663-btc-2026-09-23.json`) and on every checkable lie. `src/bridgecall.rs` — pure, tested encoders/decoders and a verifier for both hops (and Relay's
`depositNative`, recognised and refused): the calldata must decode, name the buyer's own recipient, the right destination chain
and asset, the amount asked, and a minimum out no worse than the reference floor. **What is not:** the canister executing hop 2
means operating on a **second EVM chain** (Arbitrum's RPCs, nonce and gas, and a second saved-bytes step), which the loop does not
do; and at today's liquidity hop 1 cannot carry a purchase. `configured()` therefore stays false on 4663: **no bridge is
configured, so the policy opens nothing there.** That is the honest state.

## 5. What the code settled (2026-09-23, with the code)

- `src/policy.rs`: the trigger (`candidates`, `first_fundable`, `order_for`), the vault and registry reads as calls and
  decoders (every selector and the `listing` struct checked against `cast`), and a BIP 173/350 address decoder (the listing
  names its Bitcoin address as a string). `looper::snapshot` and `looper::maybe_open` read it replicated; the registry's listing
  must name this vault. Hooked into `tick`: with nothing unfinished, the policy opens a purchase or reports why it waits.
  `Io` gained `eth_read`, `route_cost_wei` and `btc_hrp` with defaults, so other `Io`s compile unchanged. The canister's
  `route_cost_wei` is unimplemented until the rates module exists — **so on a real install the policy waits with "no route
  cost estimate", on top of "no bridge configured".**
- `open_purchase` refuses on chain 4663.
- `src/relay.rs` + `relay_venue_quote` / `relay_venue_psbt` (operator-only): the handed-in venue. A quote is accepted only while
  the purchase is not yet quoted; a PSBT only once its quote is recorded, and it must parse. Heap, not stable: after an upgrade
  the relayer hands them in again.
- Tests: `policy` 7, `bridgecall` 5, `relayquote` 2, `relay` 2, `tests/policy_loop.rs` 3 (a fake vault answering each read
  with real ABI bytes).


## 3. The venue — nothing keyless and reachable; the relayer hands the PSBT in, and cannot hurt

Checked 2026-09-23 (AAAA with `dig`, keyless with a plain GET):

| Venue | Keyless? | IPv6 | Buyer-signs PSBT | Note |
|---|---|---|---|---|
| UniSat marketplace | no (Bearer key) | yes | yes | **written permission for commercial use required (docs/phase9.md §3.1), not granted; never broadcast a real UniSat purchase** |
| Magic Eden (BTC) | reads nominally keyless | yes | yes | `api-mainnet.magiceden.dev/v2/ord/btc/runes/*` answered `503 no healthy upstream`, as docs/phase9.md §11.2 found |
| OKX | no (key + signature) | yes | yes | |
| Satflow | no (key) | yes | yes | |
| Ordinals Wallet | reads keyless | yes | unknown | its purchase flow is its app's |
| dotswap | — | **no AAAA** | — | unreachable from the canister |

So the venue is **handed in**. A relayer — the operator today, off-canister, holding whatever key a venue needs — fetches the
quote and the PSBT and submits them: `relay_venue_quote(id, quote)` and `relay_venue_psbt(id, psbt_hex, to_sign)`, stored for
the current purchase only. The loop's `Venue` is then `Relayed`, which returns what was handed in, and waits (`NoListing`) when
nothing has been. **The relayer's power is bounded by what the canister already checks:** the quote must match the purchase's
order (rune, units, recipient, our script) and our holdings, and — agent 5's change — the vault's recorded recipient and a
rate-derived cap; the PSBT is judged by `psbt.rs` against the recorded quote (runes to the order's recipient, our change back,
spend and fee under the caps, only our inputs signed). A relayer can waste time; it cannot redirect the asset or spend beyond
the recorded quote. What it **can** still do is pick a bad price inside the cap — which is why the cap must come from our own
reference rate and not from the relayer (agent 1a's rates module, agent 5's check).

## 4. What the user must decide

1. **A bridge.** Across's 4663 liquidity must grow, or a different first hop with the recipient in calldata must appear, and
   Chainflip's broker question must be answered — or accept Relay's unverifiable hop with the trust written down (bounded by
   `maxInFlight`). Until then no purchase opens on 4663.
2. **A venue's terms.** UniSat needs written permission; Magic Eden's BTC API must answer. The relayer holds whichever key.
3. **The policy's constants** — 0.05 ETH minimum, 0.5 ETH maximum, 15-minute cap margin, 5% cost ceiling — are choices.

Sources read: docs.robinhood.com/chain/bridging; api.relay.link /chains and /quote; app.across.to /api/available-routes and
/api/suggested-fees; the 4663 SpokePool's bytecode and `wrappedNativeToken()`; docs.chainflip.io vault-swaps EVM and mainnet
addresses; chainflip-sdk-monorepo `executeSwap.ts` and `enums.ts` (commit 67d43db); `dig AAAA` for every host named; this repo's
docs/phase9.md §3.1 and §11.2. All 2026-09-23.
