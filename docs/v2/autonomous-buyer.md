# etched v2: an autonomous buyer — a key nobody holds

Written 2026-09-21, before any code on this branch (`autobuyer`). A feasibility spike, not a decision record for mainnet.
Nothing here is deployed, audited, or funded. Every external fact is marked **CONFIRMED** (with the primary source read
today), **WRONG**, or **UNVERIFIED**. Web search was unavailable; sources are vendor documentation fetched directly.

## Why

v1 releases a vault's ETH to a bonded human purchaser who buys on Bitcoin. The bond bounds the damage; it does not remove the
person. The operator does not want to buy anything, front capital, or be the party holders must trust. Two designs remove the
person: an open bounty repaid on proof (permissionless fillers, heavy proof machinery, needs a market of fillers), and an
**autonomous buyer**: a program that holds a Bitcoin key and an EVM key that no person can extract, takes the vault's ETH,
bridges, buys, delivers to the winners, and posts the receipt — with nobody in the loop. The operator chose the second.
What it costs, stated once: trust does not disappear, it moves — from one person to a network's threshold, a bridge, and a
marketplace API.

## Platforms compared

| | ICP canister | Lit Protocol (PKP + Lit Action) | NEAR chain signatures | TEE service (e.g. enclave on a cloud) |
|---|---|---|---|---|
| Who could extract the key | ≥ 1/3+ of the 34-node fiduciary signing subnet colluding (threshold; key is never assembled). CONFIRMED: `key_1` lives on a 34-node fiduciary subnet | the Lit network's threshold of nodes, each in a TEE. UNVERIFIED in detail | the MPC signer set (a small, permissioned set at last reading). UNVERIFIED | the hardware vendor, anyone with a side-channel, and whoever provisions the enclave |
| Who can change the code | the canister's controllers. None if blackholed; an SNS DAO; or a timelock canister | the Action is content-addressed (IPFS CID) — immutable per CID, but WHICH CID may sign is set by whoever holds the PKP's auth; can be burned to one CID. UNVERIFIED | the contract's account keys; can be deleted to freeze it | whoever deploys the image; attestation proves which image, not that it is good |
| Reads Bitcoin itself | **Yes, natively**: `bitcoin_get_utxos`, balances, fee percentiles, `bitcoin_send_transaction` through the Bitcoin canister — no API. CONFIRMED | no; via HTTP inside the Action | no; needs an oracle/relayer | via HTTP only |
| Calls web APIs | **Yes**, HTTPS outcalls: GET/HEAD/POST; replicated (all nodes call, 2/3 agree), non-replicated (`is_replicated = false`, one node), or flexible (a committee, canister judges). 2 MB cap. CONFIRMED | yes, `fetch` inside the Action, run by each node | **no** HTTP from contracts; an off-chain relayer must act — that relayer is a person again | yes |
| Signs taproot (BIP340/341) | **Yes**: `sign_with_schnorr`, algorithm `bip340secp256k1`, with `aux = #bip341 { merkle_root_hash }` for the key-path tweak. CONFIRMED | ECDSA natively; Schnorr/taproot UNVERIFIED | ECDSA; Schnorr (ed25519) — BIP340 UNVERIFIED | anything |
| Signs EVM txs | **Yes**: `sign_with_ecdsa` (secp256k1); EVM RPC canister relays `eth_sendRawTransaction`, supports `Custom` RPC URLs for any chain. CONFIRMED | yes | yes | yes |
| Cost | ~$0.036 per production signature (26.15 B cycles, `key_1`); ~$0.014 on `test_key_1`. Outcalls priced per byte/time. CONFIRMED | per-request capacity credits. UNVERIFIED | gas | a server bill |
| Maturity for THIS job | ckBTC/ckETH run on exactly these primitives with real value; DFINITY's own guide names "Ordinals, Runes, BRC-20" as the use of the direct Bitcoin API. CONFIRMED (the sentence; not an ordinals library) | used for cross-chain automation; no native Bitcoin view | young; no HTTP | well understood, weakest trust story |
| What an auditor reviews | one canister (Rust), its PSBT verifier, its state machine, controller set-up | the Action's JS + PKP permissioning + network assumptions | contract + the relayer (off-chain, unauditable behaviour) | the image + attestation flow + ops |

**Recommendation: an ICP canister.** It is the only option that holds both keys under a threshold, reads Bitcoin without asking
anyone, calls the marketplace and the bridge itself, signs taproot with the BIP341 tweak, and can be made unchangeable. Lit is
the runner-up (no native Bitcoin view; taproot unverified). NEAR needs a relayer, which reintroduces the person. A TEE is the
weakest promise for holders: "trust this chip and whoever booted it".

## What I had told the operator from memory — checked

| Claim | Verdict | Source |
|---|---|---|
| (a) Canisters hold threshold Schnorr BIP340 keys and sign taproot spends | **CONFIRMED**, including the BIP341 `merkle_root_hash` tweak argument | docs.internetcomputer.org `guides/chain-fusion/bitcoin.md` ("`schnorr_public_key` / `sign_with_schnorr`: Taproot (P2TR) addresses"); `references/cycle-costs.md`; management-canister reference (`aux : ?{ #bip341 : { merkle_root_hash } }`) |
| (a) …read Bitcoin UTXOs natively | **CONFIRMED** | `concepts/chain-fusion/bitcoin.md` (`bitcoin_get_utxos`); note the management-canister `bitcoin_*` methods are deprecated in favour of calling the Bitcoin canister directly |
| (a) …make HTTPS outcalls | **CONFIRMED** | `concepts/https-outcalls.md` |
| (a) …sign EVM transactions with threshold ECDSA, and an EVM RPC canister exists | **CONFIRMED**; canister `7hfb6-caaaa-aaaar-qadga-cai`, NNS-controlled, `Custom` RPC endpoints for any EVM chain | `concepts/chain-fusion/ethereum.md`, `guides/chain-fusion/ethereum.md` |
| (b) Outcalls reach IPv4-only servers | **CONFIRMED**: "first tries a direct IPv6 connection; if that fails (e.g., the server is IPv4-only), it retries through a SOCKS proxy" | `concepts/https-outcalls.md` |
| (c) Canister memory is not confidential; an API key stored there is not secret | **CONFIRMED**: "Do not store sensitive data such as API keys in canisters… a malicious replica… can easily retrieve and steal your keys… in plain text" | `guides/security/https-outcalls.md` |
| (d) On-chain rune exchanges exist that a program can trade against without an API key (RichSwap / Omnity REE on ICP) | **PARTLY CONFIRMED**: REE ("Runes Exchange Environment") runs on ICP with "decentralized PSBT signing"; RichSwap is an exchange on it with Swap/Pools. **UNVERIFIED**: whether another canister may be the trader end to end (the `invoke` API page would not load), its liquidity per rune, fees, and terms. "Others exist" — UNVERIFIED, I could name none from a source | omnity.network `docs/REE/introduction`; richswap.io |

One thing I had NOT said and should have: **a 4663 RPC quorum is thin.** The EVM RPC canister gets its integrity from asking
three or more independent providers and comparing. Robinhood Chain has, as far as we know, its public RPC and dRPC. With
`Custom` endpoints the canister can ask both; two providers is a weaker check than three. Mitigation below.

## The flow

```
vault (4663)  --release(ETH)-->  buyer's EVM address  --bridge-->  buyer's taproot address
                                                                     |  buy (PSBT) at the venue
winner's / holders' Bitcoin addresses  <------ asset delivered ------+
vault (4663)  <--settle(receipt)-- buyer's EVM address (signed by the same canister)
```

1. **Trigger.** The canister's timer reads 4663 through the EVM RPC canister (`eth_getLogs` on our contracts, both RPCs, results
   must agree): a listing's vault can afford a purchase, a draw has resolved and the winner has posted a Bitcoin address (or a
   rune distribution's claims are posted). Buying happens **after** the recipients are known, so the asset never rests with us.
2. **Release.** The canister signs a 4663 transaction calling `vault.pull(purchaseId)`; the vault sends ETH **only to the
   registered buyer address** (the canister's own threshold-ECDSA address), within the cap and the in-flight limit. There is
   no human purchaser and no `declare → wait → release` to a person. The public wait is kept: `pull` is two steps an hour
   apart, so anyone can see what is about to be bought and the owner can still pause.
3. **Bridge.** ETH (4663) → BTC to the canister's own taproot address. LI.FI quoted this route on 2026-09-21 (via LayerSwap,
   0.72% at 1 ETH, ~32 min — Phase 0 finding). The canister fetches a quote, **checks it against its own bound** (an ETH/BTC
   rate from the ICP exchange-rate canister ± a stated tolerance, the recipient equal to its own address, the `to` on an
   allowlist of bridge contracts, value equal to the release), signs, sends, and then **waits for the BTC to arrive as seen by
   the Bitcoin canister**, not as reported by the bridge. If nothing arrives by a deadline, it records `bridge-stuck`, stops
   pulling for that listing, and the amount is on-chain as unaccounted — the owner's only recourse is the bridge's support. This
   is the single largest residual trust: LayerSwap is a custodial-style relayer for the minutes the transfer takes. UNVERIFIED:
   whether LI.FI's API is reachable and stable under replicated outcalls; whether an alternative (Chainflip, THORChain, Relay)
   quotes 4663 at all.
4. **Buy.** At UniSat, the documented flow is `create_bid_prepare` → `create_bid` → sign → `confirm_bid` (inscriptions and
   runes share `MarketCreateBidRequest`; runes also have batch variants). **Fields that exist** (public spec, read today):
   `auctionId`, `bidPrice`, `address` (bidder), `pubkey` (bidder), `feeRate`, `utxos` (custom inputs), and **`nftAddress` —
   "(Optional) Only for multi-address wallet, such as Xverse, hiro. Inscription receiving address"**. So a receiver different
   from the payer is a documented field; whether UniSat accepts an arbitrary third party's address there is **UNVERIFIED**
   (it is documented for a wallet's own second address). If it is refused, delivery is a second Bitcoin transaction from the
   canister to the winner — still automatic, one more fee.
   The canister **never signs a PSBT it has not checked**: every output is parsed; the seller is paid no more than the cap; the
   asset output goes to the recorded recipient; our change returns to us; the fee is within a bound; only our inputs are
   signed, with the sighash the flow expects. What it **cannot** check by itself is *that the seller's input really carries
   inscription Z or N runes* — Bitcoin consensus does not know, and the Bitcoin canister returns UTXOs, not ordinals. It takes
   the venue's word, optionally cross-checked against a second public indexer by outcall (two indexers agreeing; both are
   still off-chain opinions). **This is the second residual trust.**
   *Keyless route for runes*: if a canister can trade on an REE exchange by inter-canister call, the rune purchase needs no
   API, no key and no HTTP — the strongest version of this design. UNVERIFIED; first thing to establish in the next step.
5. **Deliver.** Inscriptions: by `nftAddress` in the buy, or a follow-up send. Runes pro rata: one runestone with edicts to
   many outputs (≈546 sats + fee per recipient; the on-chain minimum payout from v1 stays).
6. **Receipt.** The canister signs `vault.settle(purchaseId, btcTxid, assetId, amount, satsPaid)` on 4663. The vault accepts a
   settlement only from the registered buyer. There is no separate attestor: the party that bought is a program whose code is
   public; a second signer adds nothing unless it is a different program on a different network (a possible hardening: a
   second, independent watcher canister or Lit Action that must co-sign receipts).
7. **Snapshot.** Feasible in principle, expensive in practice: rebuilding balances needs every `Transfer` log of the coin via
   `eth_getLogs` through outcalls (2 MB per response, each call paid, two providers). For a coin with 10^4 transfers this is
   tens of calls and cents; for 10^6 it is thousands of calls and dollars, per draw. Costed honestly: do it for the draw's
   coin only, incrementally (keep a cursor and balances in stable memory), and publish the root on-chain exactly as the keeper
   does today — the 15-minute challenge window and the public snapshot tool stay, so a wrong root is still visible. It removes
   the keeper's word; it does not remove "the RPCs could both lie". Recommended for v2.1, not the first cut.

## What replaces the bond

A bond exists because a person could walk away with the ETH. A threshold key cannot walk. What can still go wrong is a **bug**,
a **bridge that never pays**, a **venue that lies about an asset**, or a **stuck state**. So the bound is structural:

- `release` pays **only** the registered buyer address, which is fixed at construction (changing it = a new vault).
- **Per-purchase cap** (the posted cap × count, as v1) and a **max in-flight amount** per vault: no second pull while the first
  is unsettled beyond a small limit. A bug can lose one purchase, not the vault.
- **Return after deadline**: if a purchase is neither settled nor returned by `deadline`, anyone may mark it `overdue`, which
  freezes further pulls for that vault. Funds that the buyer still holds on 4663 can be returned by anyone calling the buyer's
  public `refund(purchaseId)` — the canister signs the return transaction itself; it needs no permission because the
  destination is fixed to the vault. Funds already bridged cannot be recalled by a contract; they are either delivered later
  (settle still accepted, flagged late) or written off by the owner, visibly, forever.
- **Circuit breakers the owner keeps**: pause pulls per listing; nothing else. The owner cannot redirect funds.
- The **price cap** is still posted by a keeper in the first cut (from the venue's floor). With the snapshot work it can be
  computed by the canister from two sources; until then it is a trusted input that bounds spend, not one that moves money.

## Immutability, upgrades, kill

- **Blackholed** (no controllers): nobody can change or stop it — and nobody can fix it. A PSBT parser with a bug stays
  buggy with money in it. Not recommended for a first version.
- **SNS/DAO-controlled**: upgrades by token vote; heavy, slow, and the token is its own project.
- **Timelocked controller (recommended)**: the only controller is a small, public timelock canister; an upgrade is proposed
  with its Wasm hash, waits N days in public, and can be vetoed by a guardian key whose only power is veto. Holders see every
  change before it runs; the module hash is public, so anyone can check that the running code is the published code
  (reproducible build required). A kill switch is "stop pulling" — never "move funds".
- ICP's fiduciary subnet and the NNS remain a trust assumption: the NNS can, in the limit, replace subnet membership.

## The HTTPS-outcall hazard: N replicas, N purchases

CONFIRMED from the primary docs: a replicated outcall is sent by **every** node ("a non-idempotent POST endpoint (like 'create
order') will be called once per replica: potentially 13 times"). UniSat's spec has **no idempotency key** (searched the whole
spec). So:

- `create_bid_prepare` / `create_bid` are *safe to repeat* in effect — they return an unsigned PSBT and spend nothing — but
  13 callers will get 13 different `bidId`s and PSBTs, so **replicated consensus fails** (responses differ) unless the
  transform reduces them to nothing useful. These calls must be **non-replicated** (`is_replicated = false`) or **flexible**.
  The cost in trust: one node sees and can alter the response. That is tolerable **only because the canister verifies the PSBT
  itself** before signing: a lying node can make the purchase fail or offer a worse-but-within-cap deal, never redirect funds.
- `confirm_bid` submits a signed transaction. **Bitcoin itself is the idempotency key**: the same signed PSBT spends the same
  UTXOs, so it can confirm at most once however many times it is posted. The canister sends it non-replicated, **ignores the
  HTTP answer**, and learns the truth from the Bitcoin canister (its UTXOs spent, the recipient's output present). Read-back
  verification is the pattern the ICP docs recommend; here the read-back is consensus-grade.
- The bridge's API: quote = non-replicated + validated as above; the transaction it returns is simulated against 4663 before
  signing (the pattern pons-swarm already uses for LI.FI calldata).
- Rate limits: all canisters on a subnet share egress IPs (CONFIRMED); a keyed plan is needed for UniSat in production.

## The API key

A key in canister memory is readable by a malicious node (CONFIRMED). Options, honestly:

1. **A keyless route** (REE for runes, if canisters can trade there; direct PSBT offers from sellers) — no secret to keep.
2. **A low-privilege key**: UniSat keys authorise API quota, not funds. If stolen, the damage is someone burning our quota or
   getting us rate-limited/banned — an availability risk, not a custody risk. Rotate; keep a second key. **Acceptable.**
3. **A proxy that holds the key** — works, but the proxy operator is a trusted party again (can censor; cannot steal,
   because the canister still verifies every PSBT). Say so if used.
4. vetKeys can keep a secret from individual nodes only while it stays encrypted; the node that makes the HTTP call must see
   the header. No help here.

UniSat's terms still require **written permission for commercial use** (§3.1) whoever holds the key. An autonomous buyer does
not change that; the operator's request to UniSat stands.

## Running costs (order of magnitude; UNVERIFIED until measured)

Signatures: ~$0.04 each; a purchase needs ~4–8 (release, bridge, buy inputs, settle) → < $0.50. Outcalls: cents per purchase.
Idle canister: a few dollars a month in cycles. The bridge fee (0.7% at 1 ETH, ~5% at 0.005 ETH) dominates — batch purchases.

## What stays trusted: v1 bonded purchaser vs v2 autonomous buyer

| | v1 (bonded person) | v2 (autonomous buyer) |
|---|---|---|
| Who can take released ETH | the purchaser, up to the bond | nobody holds the key; a bug or the bridge can lose at most the in-flight cap |
| Who decides what to buy and at what price | purchaser, within the keeper's cap | public code, within the cap (cap still keeper-posted in the first cut) |
| Bitcoin custody | the operator's wallet | none at rest: bought for a known recipient |
| Receipt | an attestor's word | the buying program's own statement, checkable on Bitcoin by anyone |
| "This UTXO carries that asset" | attestor + venue | venue (+ second indexer) — **unchanged in kind** |
| Holder snapshot | keeper's word, challengeable | unchanged in the first cut; computed by the program later |
| Bridge | the purchaser's problem | **a new trusted hop** (minutes of relayer custody per purchase) |
| Platform | — | **new**: ICP's fiduciary subnet threshold and NNS governance |
| Code changes | contracts immutable | contracts immutable; canister upgrades behind a public timelock |
| Venue permission | needed | needed |
| The operator must | buy, deliver, hold keys and a bond | fund cycles, watch alerts, keep a veto key, nothing else |

## The spike on this branch (next commits)

`autobuyer/`: prove the three load-bearing capabilities as far as this machine allows — a threshold key producing a BIP340
signature that verifies for a BIP341-tweaked taproot key-path spend; an outcall to a Bitcoin testnet API; a JSON-RPC read of a
4663 fork. If the ICP SDK cannot be installed here, the taproot/PSBT half is proven off-platform and the rest is marked
unproven. `src/v2/` + tests: the vault's release-only-to-buyer and return-after-deadline rules as a sketch, touching no
mainnet constant.

## Spike results (2026-09-21, this machine, local replica only)

The ICP SDK installed without sudo (dfx 0.32.0, which now prints that `icp-cli` supersedes it). `autobuyer/canister/` is a
60-line Motoko canister; `autobuyer/spike.mjs` drives it; `autobuyer/test/` holds the tests. **Everything below ran on
127.0.0.1 with the replica's throwaway key. Nothing was deployed to ICP, no real key exists, nothing was sent to Bitcoin or 4663.**

| Proof | Result |
|---|---|
| Threshold BIP340 key → taproot address | `schnorr_public_key` returned a 33-byte key; our BIP341 tweak equals `@scure/btc-signer`'s, and equals **BIP341's own test vector** (`53a1f6e4…a343`, `bc1p2wsldez5…h4z5`) |
| Key-path spend signed by the subnet | `sign_with_schnorr` with `aux = #bip341 { merkle_root_hash = empty }` returned 64 bytes that **verify under the tweaked OUTPUT key and do NOT verify under the internal key** — the subnet applies the tweak. A standard library then finalised a 205-byte transaction from that signature |
| HTTPS outcall to a Bitcoin testnet API | UniSat's TESTNET host answered `200 {"code":0,"msg":"ok","data":{"chain":"test","blocks":5149968…` to `/v1/indexer/blockchain/info` **with no key, in non-replicated mode**; mempool.space testnet4 answered in replicated mode |
| JSON-RPC read of Robinhood Chain by outcall (POST) | `eth_chainId` → `0x1237` (4663); `eth_getLogs` on the Pons factory over the last 200 blocks → 6 logs. The public RPC was read, not a fork: the local replica's outcalls want https, and a read of the public chain proves the same thing |
| Vault rules (sketch) | `autobuyer/contracts/AutoBuyerVault.sol`, compiled by its own Foundry profile so it cannot link into v1: 8 tests pass, one a fuzz test (512 runs) of "in-flight never exceeds the limit" and "balance + outstanding is conserved" |

**Found while building.** The local replica's only BIP340 key is *named* `key_1` — the same name as mainnet's production key
(`dfx_test_key` no longer exists in dfx 0.32). A canister written for local testing therefore names the production key by
default. The real canister must take the key name from an init argument and refuse `key_1` unless an explicit mainnet flag is
set at install, the same three-fact arming idea as our other systems.

**Still unproven, precisely.** (1) Anything on a multi-node subnet: a single local node agrees with itself, so transform and
consensus failures cannot appear here (the ICP docs say so themselves). (2) UniSat's marketplace POSTs from a canister — the
key-gated endpoints were not called (no key was used, and buying needs UniSat's written permission). (3) Whether `nftAddress`
accepts a third party's address. (4) Parsing and verifying a real UniSat purchase PSBT inside a canister (needs Rust and a
recorded PSBT fixture; Motoko has no Bitcoin library to speak of). (5) The bridge leg from a canister. (6) Whether a canister
may trade on an REE exchange. (7) A reproducible build and the timelock controller.

## Effort to a testnet-complete v2 (estimate, not a promise)

Rust canister with a PSBT verifier and state machine, against Bitcoin testnet + a 4663 fork + ICP mainnet's `test_key_1`:
3–4 weeks of focused work; the vault changes and their tests: 3–5 days; bridge leg and its failure handling: 1 week; timelock
controller and reproducible builds: 1 week; then an audit that now spans two platforms. The hardest unsolved problem is not
cryptographic: it is that **"this UTXO carries that inscription / those runes" is still somebody's index**, and that the
ETH→BTC hop puts each purchase in a relayer's hands for half an hour.

## What the operator would still do, and what holders would still trust

Do: keep the canister funded with cycles (dollars a month), hold a veto key on the timelock, keep the pause switch, get the
venues' written permission, watch alerts. Not do: buy, bridge, deliver, hold a Bitcoin wallet, post a bond.
Trust: ICP's fiduciary-subnet threshold and governance; the bridge for minutes per purchase; the venue's (and a second
indexer's) statement of what a UTXO carries; the keeper's cap in the first cut; the snapshot until the program computes it.

---

# Built (2026-09-21, this branch, local only)

Everything below runs with `cd autobuyer && npm run e2e:v2`, on this machine, in about a minute. **Local ICP replica only.
Nothing deployed to ICP, no real key, nothing sent to Bitcoin or to chain 4663.**

## What is real code now, and what it does

| Piece | Where | What it is |
|---|---|---|
| Runestone decoder | `canister-rs/src/runestone.rs` | reads a transaction's OP_RETURN and says how many units of which rune go to which output — delta-encoded ids, cenotaphs, spreads, the pointer. 11 tests |
| PSBT verifier | `canister-rs/src/psbt.rs` | parses BIP 174 itself and refuses to sign unless six rules hold. 17 tests, **9 of them a hostile response** |
| State machine | `canister-rs/src/state.rs` | what to do next, decided from the two chains and never from memory. 14 tests, **12 of them an interrupted transition** |
| Bridge check | `canister-rs/src/bridge.rs` | a bridge's own offer judged against our reference rate, an allowlist, the amount and the recipient. 5 tests |
| The canister | `canister-rs/src/canister.rs` | the threshold keys, replicated JSON-RPC with a transform, the guards, the loop. Builds to a 1.1 MB Wasm |
| The vault | `contracts/AutoBuyerVault.sol` | release only to the registered buyer, cap, in-flight limit, public hour, overdue freeze, permissionless return. **15 tests**, one a fuzz |
| The whole loop | `canister-rs/tests/e2e.rs` | 14 hops from "the vault can afford one" to "the receipt is on chain", printed |

**Totals: 52 Rust unit tests, 2 end-to-end, 15 Solidity. 69, all passing.**

## Proved on the local replica

- **A taproot key nobody holds**: `taproot_key()` returns the x-only **internal** key; `evm_address()` returns the buyer's
  EVM address, derived by real secp256k1 point decompression and Keccak-256 — `0x5b0b…ebc2` for the replica's key. The vault
  is constructed with that address and can send to no other.
  **Corrected 2026-09-22.** This line previously said `taproot_key()` returns the *output* key. It does not: it returns the
  untweaked key, and the subnet applies BIP341's tweak later, when it signs. An address built on the old reading commits to
  P while every signature is made under Q, and is unspendable. No test here could have caught it — a test that signs and
  then verifies shares the assumption it is testing — and it was found only by broadcasting to signet. See
  `docs/v2/2026-09-22-signet-rehearsal.md`.
- **A guard that fires at install**: one RPC provider is refused with "two RPC providers at least: one is not a check".

## The duplicate-call hazard, answered in code

UniSat's spec has no idempotency key and a replicated outcall is made by every node, so the buy call must be non-replicated —
one node writes the response. That is safe **only** because of `psbt.rs`, and the tests say exactly how safe:

| A single lying node | What happens | Test |
|---|---|---|
| sends the runes to itself | refused: `AssetNotDelivered { wanted: 1000, found: 0 }` | `a_node_that_sends_the_runes_somewhere_else_is_refused` |
| raises the price past the cap | refused: `OverCap { spend: 80000, cap: 50000 }` | `a_node_that_raises_the_price_past_the_cap_is_refused` |
| keeps our change | refused: `OverCap { spend: 100000, cap: 50000 }` | `a_node_that_sends_our_change_to_itself_is_refused_by_the_cap` |
| delivers one unit short | refused: `AssetNotDelivered { wanted: 1000, found: 999 }` | `a_node_that_delivers_fewer_units_than_quoted_is_refused` |
| burns the runes in a cenotaph | refused | `a_node_that_burns_the_runes_with_a_cenotaph_is_refused` |
| spreads the edict over every output | refused | `a_node_that_spreads_the_edict_over_every_output_is_refused` |
| adds a second runestone | refused | `a_node_that_adds_a_second_runestone_is_refused` |
| asks us to sign a stranger's input | refused | `a_node_that_asks_us_to_sign_someone_elses_input_is_refused` |
| pays the lot to miners | refused | `a_node_that_pays_the_whole_thing_to_miners_is_refused` |
| **overpays within the cap** | **allowed** | `overpaying_within_the_cap_is_allowed_and_is_the_residual_risk_we_state` |

**So the residual is an overpay up to the cap, and availability.** Neither is custody. The sentence for the site: *a single
node can waste a purchase or make it cost up to its cap, and can never take the money or the asset.*

## A correction to what this document said before

It said the production key name is refused "unless the installer said `mainnet`". That guard was the wrong shape and is now
renamed `accept_production_key_name`. **The canister cannot tell a local replica from mainnet**: dfx 0.32's local replica
offers one BIP340 key and it is *named* `key_1`, exactly as production's is. The flag records that naming it was deliberate
and cannot be reached by leaving a default alone. That is all it does, and implying more would be worse than having nothing.

## What replaces the bond, each with its test

`pull` sends only to the buyer address fixed at construction; `capWei` bounds one purchase and expires in an hour;
`maxInFlight` bounds everything outstanding; `announce` cannot reserve more than the vault holds; the public hour cannot be
skipped; one overdue purchase freezes every new announcement; anyone may return the exact amount; a partial return is refused;
the owner may pause and write off and **can never receive funds**; the keeper may post a cap and nothing else.

## Trust: v1 against v2, as built

| | v1 (bonded person) | v2 (this branch) |
|---|---|---|
| Who can take released ETH | the purchaser, up to the bond | **nobody holds the key**; a bug or the bridge can lose at most the in-flight cap |
| Who decides what to buy, and at what price | the purchaser, within the keeper's cap | public code, within the cap, and the PSBT is verified against a quote recorded first |
| Bitcoin custody at rest | the operator's wallet | **none**: bought only once the recipient is known |
| "This transaction moves that asset" | the attestor's word | **arithmetic**: the runestone is decoded and checked |
| "That UTXO carried those runes to begin with" | the venue's word | the venue's word — **unchanged**, and the honest residual |
| Receipt | an attestor's word | the buying program's own statement, checkable on Bitcoin by anyone |
| Holder snapshot | the keeper's word, challengeable | unchanged in this cut |
| Bridge | the purchaser's problem | **a new trusted hop**, minutes of relayer custody per purchase |
| Platform | — | **new**: ICP's fiduciary-subnet threshold and NNS governance |
| The operator must | buy, deliver, hold keys and a bond | fund cycles, hold a veto key, watch alerts |

**The honest label is "trust-minimized", never "trustless".** Trust moved off one person and onto a threshold, a bridge, and a
venue's index. It did not disappear.

## Running costs (order of magnitude, still unmeasured)

~$0.04 a threshold signature; a purchase needs four to eight, so under $0.50. Outcalls, cents. An idle canister, a few dollars
a month. The **bridge fee dominates** — 0.7% at 1 ETH, ~5% at 0.005 ETH — so purchases must be batched.

## Before this could hold real money

1. **An audit of two codebases**: the Solidity vault and the Rust canister, by someone who reads PSBTs for a living.
2. **UniSat's written permission** for commercial use (§3.1). Asked for, not received. Nothing may buy until it is.
3. **The bridge chosen and its failure handled** — the largest single residual. Today LI.FI via LayerSwap is the only quoted
   route for 4663→BTC, and it is custodial for the minutes it takes.
4. **Inscriptions**, which need their own design: an inscription's provenance is an indexer's word, where a rune's movement is
   arithmetic. Runes first for exactly that reason.
5. **The timelock controller and a reproducible build**, so the running code is provably the published code.
6. **A multi-node subnet**: transform functions and consensus failures cannot appear on a single local node, and the ICP docs
   say so themselves.
7. ~~**A real Bitcoin broadcast and read-back**~~ — **done on signet, 2026-09-22**. A node accepted a key-path spend signed
   by the threshold key, and signet block 323256 contains it (txid `3d3ed273…7f9f`); the read-back is from the block's own
   txid list. Still outstanding within this item: on mainnet the read-back should be the Bitcoin canister's consensus view,
   not the two-explorer outcall this branch uses. Full account, including what it did **not** prove, in
   `docs/v2/2026-09-22-signet-rehearsal.md`.

## What is still fixture or unverified

The bridge's reply and the venue's reply are fixtures behind traits. No marketplace call was made. A Bitcoin transaction
**has** now been broadcast and mined, on signet — but a self-send, not a purchase: no marketplace, no seller's pre-signed
input, no PSBT we did not construct, and no runes (`docs/v2/2026-09-22-signet-rehearsal.md`). **The EVM half remains
entirely unexercised against a chain**: `sign_evm` has no callers, nothing builds or sends a 4663 transaction, and the
recovery id an EVM signature needs is not derived anywhere — the same shape of invisible assumption the taproot mislabel
was, and it will stay invisible until a node rejects it. `observe()` returns a default rather than reading the vault's logs — the shapes are settled and the state machine is
tested against every value they can take, but the two readers are the next piece of work. Whether a canister may trade on an
REE exchange (the keyless rune route) is still **UNVERIFIED**, and it remains the strongest available version of this design.
