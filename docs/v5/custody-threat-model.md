# Custody threat model: every way the items and the money could leave

Written 2026-09-23 on branch `custody-security` (from `9022920`, the merge of `v4-pools`). Docs, tests and unwired Rust only: no
contract, canister, keeper or web file was edited, nothing was deployed, and no key was handled. The fixes this document calls for
are specified in `docs/v5/custody-hardening.md`; what must be true before any value enters custody is `docs/v5/launch-gates.md`.

`docs/security/threat-model.md` (v1) and the actor table in `docs/audit/README.md` (v2/v3) still stand. This document is narrower:
**custody**. That is the items Etchwrap holds for other people, the money that is on its way to becoming items, and the wrapped
tokens whose value depends on both. A custodian that holds blue-chip items is a target. So this document assumes that someone
capable is trying.

## 1. What is in custody, and who can move it

| Asset | Where it sits | What moves it | Symbol |
| --- | --- | --- | --- |
| Bitcoin inscriptions backing `Wrapper` (wFROG) | the canister's taproot address on derivation path `b"custody"` (`autobuyer/canister-rs/src/canister.rs:336`) | the canister's Schnorr threshold key on that path. Only two code paths sign with it: a redemption delivery (`src/redeem.rs:318-324`) and the return of a refused deposit (`src/frogflow.rs:944-948`) | **B** |
| Plain sats that front frog purchases | path `b"float"` (`src/frognet.rs:22`) | frog buys (`src/frogflow.rs`, judged by `src/frogbuy.rs`) | **F** |
| BTC bridged in for a vault purchase | path `b"btc"` (`src/canister.rs:334`) | `sign_purchase` and the loop, against a recorded quote (`src/psbt.rs`) | **F_b** |
| ERC-721s backing each `NftWrapper` (wAPE, …) | the canister's EVM address on chain *c* (1, 8453, …): one ECDSA key, path `b"evm"`, the same 20 bytes on every chain (`src/canister.rs:425`) | today, nothing: the delivery path is a spec (`docs/v4/nft-wrapper.md` §5.3). Once built, the same key signs `transferFrom` | **N_c** |
| ETH the canister holds on chain *c* | the same EVM address | `nft_sign` (`src/nftnet.rs:275`), and later the Relay deposit | **E_c** |
| ETH in 4663 contracts that pay the canister | `ItemQueue.queued`, `MintDesk` escrow, `FrogCurve` in `Graduating`, `NftRemoteVault`, `AutoBuyerVault` | `ItemQueue.recordItem` (`src/curve/ItemQueue.sol:181`), `MintDesk.fulfill` (`src/curve/MintDesk.sol:79`), `NftRemoteVault.pull` (`src/nft/NftRemoteVault.sol:138`), `AutoBuyerVault` pull: each pays the canister, **on the canister's word**, bounded by a keeper cap or `maxInFlight` | **Q** |
| ETH paired against wrapped tokens | `LiquidityLock`'s Uniswap v4 positions (wX/ETH shared pools, coin pools) | nobody can withdraw it (`docs/v3/pools.md` §3). **Anyone holding unbacked wX can sell it into those pools** | **P** |
| The wrapped tokens themselves | holders | each is a claim on **B** or **N_c**. An unbacked mint dilutes every holder | — |

**Custody value** `C = B + Σ_c N_c`. **Value reachable from custody** is `C + F + F_b + Σ_c E_c + Q + P`. The last two terms matter
because the canister is also the only minter of every wrapper (`src/Wrapper.sol:170`, `src/nft/NftWrapper.sol:120-138`). Anything
that can sign as the canister's EVM key on 4663 can mint unbacked wX, sell it into **P**, and claim **Q**.

A reference point for the numbers below, from `docs/v3/curve.md`: one frog costs ≈ 0.65 ETH all-in and one ape ≈ 5.1 ETH. After 20
graduations a shared pool holds ≈ 18 ETH (frogs) or ≈ 13 ETH (apes).

## 2. Scales

- **Likelihood.** *Low*: needs a rare event, a large expense, or several independent parties to fail together. *Medium*: one hot
  key, one service or one plausible bug is enough. *High*: expected within a year of operation unless fixed.
- **Impact** is the **maximum value at risk**. It is given twice: **today**, meaning the code at `9022920` with no caps, and
  **with H**, meaning after the hardening in `docs/v5/custody-hardening.md` (H1–H14) at launch stage 1 of `docs/v5/launch-gates.md`
  (12 items per collection, 40 ETH of custody overall, 6 outbound items a day per collection).

## 3. Summary, ranked by expected loss today

| # | Threat | Likelihood | Max at risk today | Max at risk with H | Main fixes |
| --- | --- | --- | --- | --- | --- |
| T6 | **The guardian's pause does not stop the signing paths** (new finding) | Medium, whenever the operator is compromised | every Σ E_c, plus what T5 reaches, *during* the incident | 0 once paused | H1 |
| T5 | **Operator hand-ins**: the NFT target cap is the operator's word (new finding) | Medium | **Σ E_c**, the whole ETH balance on every EVM chain | ≤ one per-item cap, per collection policy | H1, H2 |
| T10 | **Provider collusion in pinned reads** (4663 has one independent operator today) | Medium | **B** (deliveries have no rate limit), plus 50 unbacked deposit mints a day | 6 items a day per collection, then a freeze | H3, H4, H9, H10 |
| T7 | **Wrapper mint abuse** (the canister's attestation is trusted on chain) | Low–Medium; it follows from T1, T3, T16 or a canister bug | **P + Q**, and dilution of every holder | P + Q at the canister; bounded on chain only by H11 | H2, H3, H11 |
| T1 | **Upgrade by a compromised controller** | Low after the handover; **High before it** | everything: **C + F + Σ E_c + P + Q** | the same, but 72 h in public, vetoable | H5, H6, H7 |
| T8 | **Redemption re-roll and refund races** (includes a new finding: leaked delivery bytes) | Medium | **1 item per race**; unbounded if the leaked-bytes window is exploited systematically | 1 item per race, and rare | H4, H8 |
| T4 | **Keeper compromise: caps and records** | Medium | `(cap − floor) × items` of **Q** per cap hour; 1 redeemer's token per false `recordDelivery` | bounded by H12's step limit | H12, H3 |
| T13 | **Cycles exhaustion freezes custody** | Low–Medium | nothing is stolen; **C is frozen** until a controller change, and in-flight state is lost | alarm days ahead | H13 |
| T9 | **Reorgs** on Bitcoin, Ethereum, Base or 4663 | Low | 1 item per reorged ticket (4663); 1 unbacked mint per reorged deposit | ≈ 0 with finality gates | H9 |
| T3 | **Subnet or threshold-key compromise** | Low | everything the keys control: **C + F + Σ E_c**, then **P + Q** by minting | the stage cap: this is what caps are for | H2 |
| T14 | **Frontend or DNS compromise** | Medium | every redemption and approval made during the compromise | the same; shortened by monitoring | H14 |
| T2 | **Timelock bypass** | Low | as T1 | as T1 | H5, H6 |
| T11 | **OpenSea zone, extraData, royalty receiver, transfer validators** | Low | gas on a reverted fill; up to `max_fee_bps` of one fill; a stuck NFT | the same | configuration |
| T15 | **The Safe (EVM owner) compromised** | Low | after 7 public days: the keeper role (T4) and listing addresses | the same | watcher |
| T16 | **One EVM key everywhere; rehearsal signing oracles** | Low | as T3, for whatever a reused key holds | 0 by ceremony | H10, gates |
| T12 | **drand failure or bias** | Low | nothing stolen; redemptions stall; bias skews *which* item leaves | the same | accepted |

The rest of this document takes each row in turn, with the evidence.

---

## T1. A compromised controller upgrades the canister

**Path.** Any controller of the buyer canister can `install_code` a module that signs anything with every derivation path. That
covers `b"btc"`, `b"custody"`, `b"float"` and `b"evm"` on every chain. The IC enforces only that the caller is a controller. It
checks nothing about what the new module does.

**Likelihood.** High *until the handover* (step 6 of `docs/deploy/canister-mainnet.md`), when the deployer identity is the
controller and any copy of it is a master key. After the handover it is Low: it then needs a proposer, plus fewer than k guardians
vetoing within 72 h, plus nobody noticing (T2).

**Impact.** Everything: **C + F + F_b + Σ E_c**. The new code can also mint wrappers, so add **P + Q**. The IC cannot tell a
malicious upgrade from an honest one.

**Existing mitigation.**
- Every update method has a caller guard: `only_controller` (`src/canister.rs:88`) and `only_operator` (`src/canister.rs:98`).
  The test that reads every `#[ic_cdk::update]` checks it (`src/canister.rs`, tests; `src/nftnet.rs:410`).
- The timelock (`autobuyer/timelock-rs`) is the only controller after the handover. Upgrades are public for ≥ 72 h
  (`rules.rs:14`), carry a published module hash and argument, and k = 2 guardians can veto them. It upgrades and never reinstalls
  (`timelock-rs/src/canister.rs:168`). Once installed it is blackholed.
- A reproducible Docker build lets anyone check the module hash (`docs/v2/timelock-controller.md`, "Reproducible build").
- The upgrade argument cannot change the canister's identity: chain, network, key, vault and wrapper
  (`src/canister.rs:159-170`).

**Gap.**
1. **The upgrade argument is code.** `restore_init` takes `rpc_urls`, `btc_api_urls`, `operator` and all of `evm_chains` from the
   argument (`src/canister.rs:163-167`). `evm_chains` includes each chain's providers, `fee_recipients`, `max_fee_bps` and the
   Seaport code hash. A proposal whose wasm hash is unchanged but whose argument swaps providers to ones an attacker controls is
   as dangerous as new code (T10). The watcher (`scripts/timelock-watch.mjs`) reports hashes. It does not decode or diff the
   argument.
2. **Nobody is paid to read a proposal.** The delay protects only the people who look.
3. **Before the handover** there is a window in which one identity controls a canister whose addresses are already published
   (step 3 of the runbook comes before step 6).

**Fix.** H5 (the watcher decodes and diffs every upgrade argument against the running `Init`, and flags any change to providers,
operator or `evm_chains` as critical); H6 (a published build-and-announce rule: no proposal without its commit, its reproducible
hash and a diff summary in a public place at the moment it is proposed); H7 (install, handover and blackhole in one witnessed
sitting, and no value sent to any canister address until `dfx canister info` shows the timelock as sole controller). Custody caps
(H2) are what bound the damage if everything else fails.

## T2. The timelock is bypassed

**Paths.**
(a) The buyer still has a second controller: the deployer was never removed, or a later `update_settings` added one.
(b) The timelock is not blackholed, so its own controller can reinstall it with a shorter delay.
(c) The rehearsal build is guarding production. `timelock-rehearsal.wasm` has a 10-second floor (`rules.rs:16`).
(d) A timelock bug admits an early `execute`, a reinstall, or a proposal from a non-proposer.
(e) A proposer colludes with k guardians. This is the design's stated limit.
(f) NNS governance acts on the subnet. The IC's own governance is out of our control.
(g) A module over 2 MiB is proposed through a path the timelock does not have, so someone adds a person as controller "for the
    chunked install". The buyer was 2.53 MB raw at the custody branch and only its gzip fits in one message
    (`docs/v3/wrapper.md`, "Found", 4).

**Likelihood.** Low if the ceremony is followed and verified. (a)–(c) are ceremony errors and cost nothing to check. (g) grows
more likely with every module added.

**Impact.** As T1.

**Existing mitigation.** The rules are tested for every row (`rules.rs`, 8 tests). Two replica rehearsals pass
(`rehearse.sh`, `rehearse-roles.sh`). The runbook's "What anyone can verify afterwards" section (`docs/deploy/canister-mainnet.md`).
The watcher has `--expect-*` flags and alerts on a rehearsal build.

**Gap.** Nothing continuously checks that the buyer's **controllers** are exactly `[timelock]` and that the timelock's are `[]`.
The runbook checks this once, by hand. A chunked-install path does not exist.

**Fix.** H5 extends the watcher to read `/canister/<id>/controllers` and `module_hash` through `read_state` (certified) on every
poll, and to alert on any change that has no matching `Executed` event. Chunked install through the timelock
(`upload_chunk` / `install_chunked_code`, with the timelock as `store_canister`) must exist before the module crosses 2 MiB
gzipped. A launch gate records the module's size.

## T3. The subnet or the threshold key is compromised

**Path.** The keys are held by the subnet that hosts `key_1` (the 34-node fiduciary subnet). A supermajority of its nodes
colluding, or a flaw in the threshold protocol, yields signatures for **every** derivation path of **every** canister using that
key. Separate derivation paths do **not** help here: whoever holds the master key can derive them all.

**Likelihood.** Low. DFINITY's threshold ECDSA and Schnorr, and the Bitcoin integration, were audited (Trail of Bits, NCC Group;
`docs/audit/README.md`, "Who could audit this"). The same key secures ckBTC and ckETH, which hold far more than Etchwrap will.

**Impact.** **C + F + F_b + Σ E_c**. The EVM key is also every wrapper's minter, so add **P + Q**.

**Existing mitigation.** None of ours. This is the IC's security, and it is out of scope by design.

**Gap.** Nothing limits how much sits behind the key.

**Fix.** H2, custody caps. The only honest answer to "the key could be stolen" is "then this much is at risk, and no more". The
staged caps in `docs/v5/launch-gates.md` exist for this threat above all. H11 (an on-chain mint rate limit) keeps a stolen EVM key
from turning into an unbounded mint in a single block.

## T4. The keeper is compromised: caps, records, roots

**What the keeper posts and what each post moves.**

| Keeper act | Code | What a malicious value does | Bound today |
| --- | --- | --- | --- |
| `SweepVault.postCap(weiPerWrapped)` | `src/SweepVault.sol:199` | Any non-zero value, no step limit. Through `CurveFactory.freshCap` it becomes the queue's `itemCap`, the opening target, the pool add gate and the center | 1-hour TTL on both clocks (`:207`); pool adds guarded to within 10% of the pool's price (`docs/v3/pools.md` §3c) |
| `NftVaultBase.postCap(capWei, ref)` | `src/nft/NftVaultBase.sol:202` | Sets the per-NFT ceiling for `NftSweepVault` sweeps and `NftRemoteVault` announcements | 1-hour TTL |
| `ItemQueue.recordItem` pays up to `maxItemCost()` | `src/curve/ItemQueue.sol:181` | With an inflated cap, the canister, buying honestly from a colluding seller, pays the inflated price from **Q** | per item: `cap × (1 + buffer)`. Per hour: every item the queue can pay for |
| `Wrapper.recordDelivery(id, txid)` (keeper *or* custodian) | `src/Wrapper.sol:267-279` | Marks a resolved ticket delivered with a made-up txid. The redeemer's token is gone, no frog is sent (the canister then refuses: `Delivered` → Refuse), and the frog stays at custody **uncounted**: `inscriptionHeld = false`, `held--` | one redeemer's token per false record; no bound on the number of records |
| `WrapClaim.open` roots, `Distributor` snapshots | as v3 | misallocate within one frog's tokens | a 15-minute owner cancel (`docs/v3/wrapper.md`, step 2) |
| Hands orders, quotes, PSBTs and offers to the canister | see T5 | — | the canister's checks |

**Likelihood.** Medium. The keeper is a hot service (Railway) holding an EVM key and, in practice, the operator identity too.

**Impact.** **Q** at `(cap − floor)` per item, repeated for every item bought while the cap is inflated. **P**: the cap steers where
new pool depth goes but cannot move locked liquidity. One redeemer's frog per false `recordDelivery`: griefing that leaves an
orphaned frog, not theft.

**Existing mitigation.** Caps expire after one hour on both clocks. The keeper role can be rotated after 7 days
(`ListingRegistry.proposeRoles`, `src/ListingRegistry.sol:235`, `CHANGE_DELAY` `:32`), and it cannot be an auto-buyer (`:238`).
The canister buys only real items of the collection: membership proofs and ord for frogs, `ownerOf` and the Seaport checks for NFTs.

**Gap.** `postCap` takes any value, with no bound on the step from the last cap and no ceiling. The keeper's `recordDelivery` has no
cross-check against Bitcoin, and after the custodian can record its own deliveries (`docs/v3/wrapper.md`, "Custody on its own
key") nothing needs the keeper to have this power.

**Fix.** H12 bounds the step: a new cap may differ from the last fresh one by at most ±20%, and at most one post per 10 minutes.
This is a Solidity change for the contracts' owner. Separately, remove the keeper's `recordDelivery` power once the custodian
records its own deliveries on mainnet. That is a Solidity change to `Wrapper.sol`, also for the contracts' owner. The watcher alerts
on any `Delivered` event whose txid Bitcoin does not show paying the ticket's destination (H3's canary).

## T5. Operator hand-ins: orders, quotes, PSBTs, offers, targets

The operator (the relayer, run by the keeper service) hands the canister what it cannot fetch keylessly. Each hand-in, and what
bounds it:

| Hand-in | Entry point | Bound | Residual |
| --- | --- | --- | --- |
| rune purchase quote and PSBT | `record_quote` (`src/canister.rs:1350`), `sign_purchase` (`:1280`), `relay_venue_*` | the listing's `btcVault`, rune and units; spend ≤ released ETH at the reference rate + 3% (`docs/v2/trust-points.md` §1); `psbt::verify`; ord-clean inputs | 3% plus overpaying within the cap |
| frog offer (PSBT) | `frog_offer` (`src/frognet.rs:394`) | `frogbuy::judge`: the inscribed sat lands on custody; spend ≤ `cap_sats` from the sink's keeper cap; fee ceiling; `MAX_PURCHASE_WEI` (`src/frogbuy.rs:69`) | T4's cap |
| membership proof | `frog_proof` (`src/frognet.rs:415`) | checked against the listing's root at every use | none |
| refuse a deposit | `refuse_deposit` (`src/frognet.rs:426`) | only a deposit waiting for its proof; it is only ever returned to its own sender, 10 a day (`src/deposits.rs:24`) | griefing a depositor, who gets the frog back |
| **NFT target and its cap** | **`nft_set_target(chain, collection, token_id, cap_wei)`** (`src/nftnet.rs:247`) | **none**: any collection, any token, any cap, on any configured chain | **E_c on every chain** |
| Seaport order | `nft_preview`, `nft_sign` (`src/nftnet.rs:266`, `:275`) | `seaport::verify` against the target: total ≤ the target's cap (`src/seaport.rs:356`); fee recipients allowlisted or ERC-2981 (`:347`); simulation at the pin | the cap |

**The new finding.** `docs/v4/eth-nft-canister.md` §6 rule 5 says the cap "was recorded … **before** the order was handed in, and the
order never sets it". That holds. But the cap is recorded **by the same operator** that hands in the order, through
`nft_set_target`, and nothing bounds it. The document says "the cap is the keeper's (or, once B's registry exists, the
registry's)". Today the registry is not read. So a stolen operator key can:

1. deploy any ERC-721 on chain *c*, mint itself one token, and list it on Seaport for the canister's whole balance **E_c**;
2. call `nft_set_target(c, that collection, that token, E_c)`;
3. call `nft_sign`, then `nft_send`.

Every check passes: the canonical Seaport, an open order, the exact token, delivered to us, total ≤ cap, no fee recipients, not
expired, fillable, signed by the offerer. The canister pays **E_c** for a worthless NFT.

**Likelihood.** Medium: one hot identity on one server.

**Impact.** **Σ E_c**, all the ETH the canister holds on every EVM chain. Custody items are not reached this way (no outbound NFT
path exists yet).

**Fix.** H2: `nft_set_target` refuses a collection the custody policy does not list, and a cap above the policy's per-item
maximum for that collection. The per-item maximum is itself bounded by the registry's fresh keeper cap once the registry carries
one (`docs/v4/eth-nft-canister.md` §9). H1: the guardian's pause stops `nft_sign` and `nft_send` (T6).

## T6. The guardian's pause does not stop the signing paths (new finding)

**What the documents say.** `docs/deploy/key-ceremony.md`, stolen operator: "**Do:** a guardian pauses the buyer (`pause_target`;
the operator cannot resume)". The audit actor table says the operator's power is stopped by "a guardian's pause (which the operator
cannot lift)".

**What the code does.** `PAUSED` is read in exactly four places, all inside `tick` (`src/canister.rs:1001`, `:1024`, `:1040`,
`:1051`). None of these update methods reads it:

- `sign_purchase` (`:1280`): signs a verified PSBT against a recorded quote;
- `record_quote` (`:1350`) and `open_purchase` (`:1067`);
- `nft_sign` (`src/nftnet.rs:275`) and `nft_send` (`:325`): sign and broadcast a Seaport fill;
- `nft_set_target` (`:247`);
- `frog_offer`, `frog_proof`, `refuse_deposit` (these only stage input for `tick`, which is paused, so they are harmless);
- `rehearse_sign_sighash`, `rehearse_sign_evm`, `rehearse_broadcast_*` (these refuse on a production install for other reasons).

So after a guardian pauses, a stolen operator key can still run T5's drain end to end. It can also still sign a purchase PSBT
(bounded by the recorded quote, `docs/v2/trust-points.md` §1). The pause stops the *loop*. It does not stop the *operator*.

**Likelihood.** Medium: it fires exactly when the pause is needed.

**Impact.** Everything T5 reaches, during the incident, after the guardians believe they have stopped it.

**Fix.** H1: one guard, `not_paused()`, called first in every update method that signs, sends or records, plus a test that reads
every `#[ic_cdk::update]` that signs and asserts it. The exact diff is in `docs/v5/custody-hardening.md`. This is the cheapest fix
in the whole document and it comes first.

## T7. Wrapper mint abuse: the canister's attestation is trusted

**Path.** The chain cannot see Bitcoin, Ethereum or Base. So every wrapper takes the custodian's word for arrivals:

- `Wrapper.wrapDeposit(inscriptionId, proof, txid, vout, to)` (`src/Wrapper.sol:170`) mints `1e18` to `to` for any collection member
  not already held, once per outpoint;
- `NftWrapper.mint` / `mintDeposit` (`src/nft/NftWrapper.sol:120-138`) mints `1e18` for any token id not currently held, once per
  `(tokenId, txHash)`;
- `ItemQueue.recordItem(cost)` and `MintDesk.fulfill(id, cost)` pay the custodian `cost` once it has minted a unit to them.

Anything that can sign as the canister's EVM key on 4663 (T1, T3, T16, or a canister bug in the mint path) can:
1. mint wX for every collection member not held (up to the collection's size: 10,000 for many);
2. sell it into the wX/ETH pools (**P**);
3. mint to the queue and the desk and claim `cost` up to each one's cap (**Q**).

**Likelihood.** Low–Medium. It needs a compromised or buggy custodian, but it turns any such compromise into a loss much larger
than custody itself, because **P** belongs to third parties.

**Impact.** **P + Q**, plus dilution of every holder of that wrapper.

**Existing mitigation.** Membership proofs, one mint per item and per source, and `inscriptionHeld` / `itemOf`. The canister
limits itself to 50 deposit mints a day (`src/deposits.rs:20`). The NFT deposit spec proposes 20 a day (`docs/v4/nft-wrapper.md`
§4, check 6). Both limits live **in the same canister** whose compromise is the threat.

**Gap.** Nothing on chain bounds the mint rate. A compromised minter mints in one block.

**Fix.** H11 (Solidity, for the contracts' owner): a per-wrapper mint rate limit in the contract, e.g. at most `M` mints in any
24 hours on both clocks, with `M` fixed at construction. It limits what a stolen key can mint before the guardians and the market
react. H2 and H3 bound the canister's own mint path. The proof-of-reserves page (launch gates, G5) makes an unbacked mint visible
within an hour.

## T8. Redemption re-roll and refund races

| Race | Where | Status |
| --- | --- | --- |
| A Bitcoin delivery sent near `deliverBy` **and** a refund on 4663: the redeemer gets both | `docs/v3/wrapper.md` "Built" 6 | **mitigated**: `delivery::decide` refuses inside a 24-hour margin on either clock (`src/delivery.rs:16`, `:85-116`) and is consulted before every send and re-send (`src/redeem.rs:229-241`) |
| **Leaked signed bytes**: a delivery broadcast, then evicted from mempools, then `Refuse`d on re-check. The record says "not sent again" (`src/redeem.rs:236`, `:262-266`) | `src/redeem.rs` | **fixed by H8 at `72531db`** (it was a new finding: the bytes were public while they were in a mempool, and anyone who saved them could re-broadcast them **after** the refund, the redeemer then holding the frog **and** the token). Now the record becomes `Conflicting` and the fee input is spent back to custody before it settles. Left: a redeemer who pins the leaked bytes with a large child can still win the race (`custody-hardening.md`, H8 note, point 9) |
| A refused deposit's return bytes, the same way | `src/frogflow.rs` | lower stakes: the frog is the sender's own. H8 (`72531db`): a return unseen for 72 h is conflicted out, then built afresh |
| NFT delivery with `safeTransferFrom` to a contract that reverts: it forces a refund, and the redeemer re-rolls | `docs/v4/nft-wrapper.md` §5.3 | spec says `transferFrom`; **not built** |
| A collection with transfer restrictions (Limit Break validators, block lists) refuses delivery to some destinations: the redeemer re-rolls | `docs/v4/nft-wrapper.md` §9.6 | open, and made worse by T11's validators |
| The keeper's false `recordDelivery` | T4 | open |
| One skipped drand round holds the queue for 24 h | `docs/v3/wrapper.md` "Built" 4 | accepted |

**Likelihood.** Medium for leaked bytes. Evictions happen whenever fees spike, and "save every tx and replay it after refunds" is
cheap to automate. Low for the rest.

**Impact.** One item per event. Leaked bytes can be harvested systematically: every eviction followed by a refusal is one free
item.

**Fix.** H8 (conflict-out). When a redemption whose bytes were ever broadcast ends in `Refused`, the custodian spends the
delivery's **fee input** back to custody at a higher fee (BIP125 replacement: custody's inputs signal RBF with sequence
`0xfffffffd`, `src/custody.rs:23`). Once it confirms, the old bytes can never be mined. The ticket may be refunded only after
that, and the custodian knows it: the refund is on chain, and so is its own conflict. H4 verifies that the conflict transaction
pays only custody. For EVM deliveries (once built): burn the nonce with a 0-value self-transfer at a higher fee. Deliver NFTs
with `transferFrom` only, and H4 refuses any other selector.

## T9. Reorgs

| Chain | What the canister acts on | Depth used | Risk |
| --- | --- | --- | --- |
| Bitcoin | deposits and frog buys at 6 confirmations (`src/frogbuy.rs:19`); deliveries "confirmed" at 1 | 6 / 1 | A >6-block reorg undoes a deposit after its mint: unbacked wX. Cost ≫ one frog. Low |
| Ethereum (1) | `nft_status` → `Arrived` at a pin 1–2 buckets behind the head (97–194 s; `docs/v4/eth-nft-canister.md` §3) | not finalized | A mint on 4663 against an arrival that reorgs out would be unbacked. The mint spec requires the `finalized` block (§5.1); **not built** |
| Base (8453) | same, 128–256 s | "unsafe" head region | as above. Finality is ≈ 20–30 min (L1 batch) |
| 4663 (Nitro) | tickets, sinks, the desk, deposits' EVM side: pins ≈ 107–214 s behind the head | **sequencer soft-final** | A redeem or resolve that the sequencer reorgs out after the canister delivered means a frog left for a burn that no longer exists. The sequencer is one operator. Low, but the loss is a whole item |

**Fix.** H9: every **outbound** decision (a delivery, a return, a mint on 4663) reads the facts it relies on at a block at or
below that chain's `finalized` tag (4663: Nitro's L1-finalized block; Ethereum: the beacon chain's finalized checkpoint; Base:
L1-finalized). Every provider must agree on its hash. This costs about 15–30 minutes of latency against a 7-day deadline. H4's
verifier takes a `finalized` flag and refuses without it.

## T10. Providers collude, or one operator serves them all

**Path.** Every EVM read is pinned and believed only when **all** configured providers agree (`docs/v2/pinned-reads.md`). Bitcoin
facts come from esplora explorers that must agree, and from ord (ordinals.com) for inscription locations. So the integrity claim
equals the independence of the providers:

- **4663**: `rpc.mainnet.chain.robinhood.com` and `rpc.ordofi.network` (since 2026-09-24, when `robinhood.drpc.org` stopped serving
  pinned reads: docs/v6/phase1-scope.md C8). ordofi publishes nothing about its infrastructure; if it routes to Robinhood's nodes,
  the "two providers" are one operator. (Written 2026-09-23 with dRPC, an aggregator, and the same caveat.)
- **Bitcoin**: mempool.space and blockstream.info. There is one ord.
- **Ethereum and Base**: two IPv6 providers each (`docs/v4/eth-nft-canister.md` §1).

**What colluding 4663 providers can forge, and what it costs.** A `Resolved` ticket naming an attacker's taproot address and any
held frog. `delivery::decide` then says Send, and custody delivers. There is no rate limit on deliveries (`src/redeem.rs:419`
walks every owed ticket), so **B** can be drained at the speed of Bitcoin blocks. Forged sink and desk state lead to mints to the
wrong place, and forged listings to purchases paying elsewhere (bounded by `btcVault` being read from the same forged source).

**What colluding Bitcoin sources can forge.** A deposit that never happened. The canister then mints unbacked wX to the attacker,
at 50 a day (`src/deposits.rs:20`).

**Likelihood.** Medium. Few providers, shared infrastructure (all sit behind Cloudflare), and a lie is cheap if the one operator
behind both is compromised.

**Impact.** **B** in hours (today). 50 × frog value a day in unbacked mints.

**Existing mitigation.** Agreement across all providers and all nodes; pins by block hash; checkpoints; distinct URLs
(`src/canister.rs` `validate`, `src/chains.rs:78-108`).

**Gap.** Nothing requires independent operators. Deliveries have no rate limit. Nothing checks for consistency across reads (a
ticket's `Redeemed` log, the burn's `Transfer` to zero, `totalSupply` against `held − openTickets`).

**Fix.** H3 (an outbound rate limit per collection and overall, with a tripwire that freezes custody when demand exceeds it);
H9 (finality); H10 (≥ 3 providers from ≥ 3 independent operators on every chain the canister acts on, one of them a node the
project runs; a launch gate); H4 (a delivery's destination must match the ticket *and* the ticket's `Redeemed` event at a
finalized block). A forged ticket then has to forge a consistent log too, and even a perfect forgery drains at most the daily limit
before the tripwire and the canary (H3) fire.

## T11. OpenSea zone, extraData, royalties and transfer validators

**Paths and bounds** (`docs/v4/eth-nft-canister.md` §12.4):
- `extraData` cannot move the NFT or change the price: the recipient, the value and the consideration are ours or signed by the
  offerer. At worst it turns a fill into a revert (the gas is lost).
- The zone is pinned by address and code hash per chain. OpenSea's controller can change the zone's signers. That can stop fills,
  not steal.
- **The royalty receiver** is read from ERC-2981 at the pin (`src/seaport.rs:347`). A collection owner can point it at themselves.
  The canister then pays up to `max_fee_bps` of a fill to them, within the cap.
- **Transfer validators** (Limit Break, substandards 7/8/9) can refuse transfers. That can make a delivered or bought NFT stuck, or
  refuse a delivery (T8's re-roll).

**Likelihood / impact.** Low. Gas; `max_fee_bps` of one fill; a stuck item.

**Fix.** Configuration: per-chain `max_fee_bps` ≤ 1,000 (10%), and prefer collections whose `transferFrom` is plain ERC-721. The
open question stays open: does OpenSea issue zone signatures to an unauthenticated fulfiller (`docs/v4/eth-nft-canister.md` §12.7)?

## T12. drand fails or is biased

**Paths.** evmnet stops, so pinned rounds never arrive: tickets queue behind the first unresolved one, and after 24 h anyone can
re-pin (`src/Wrapper.sol:219-226`). Or the League of Entropy's threshold is compromised, so randomness can be predicted, so a
redeemer can time burns to draw the best item.

**Likelihood / impact.** Low. Nothing is stolen by a halt: tokens stay burned on open tickets, which refund only from `Resolved`
(accepted, `docs/v3/wrapper.md` "Built" 8). Bias moves value between redeemers, bounded by the spread between the best item and the
mean.

**Fix.** Accepted. The beacon is immutable per wrapper. Monitoring alerts when no round has been proven for 1 hour while tickets
are pinned.

## T13. Cycles run out and freeze custody

**Path.** The buyer runs out of cycles. It freezes first (below its freezing threshold). If it is never topped up, the IC
**uninstalls** it: the module and all its state are deleted. The canister id, its controllers and therefore its threshold keys
and addresses survive, because keys derive from the canister id and the path.

**Consequences.**
- While frozen: no deliveries. Tickets refund after 7 days. Tokens come back, and the items stay at their addresses. Safe.
- After an uninstall: the timelock can only `upgrade` (`timelock-rs/src/canister.rs:168`), and an upgrade needs an installed module
  (to confirm by a drill: G8). Recovery is `propose_controllers` to a new controller, 72 h, and then a fresh install. That install
  **loses every in-flight record**: signed-but-unconfirmed delivery bytes (T8, leaked bytes), `Recording` EVM transactions, the
  per-chain nonce lines, deposit records. It must be treated as an incident.
- **State growth.** `pre_upgrade` serialises the whole state with candid (`src/canister.rs:221-240`), and the timelock upgrades with
  `skip_pre_upgrade = false` (`timelock-rs/src/canister.rs:168`). `PURCHASES`, redemptions, frogs and deposits grow for ever (only
  `LOG` is bounded, at 500). Past the upgrade instruction limit the buyer becomes **un-upgradable**: its code is frozen as it is,
  including any bug.

**Likelihood.** Low–Medium. The watcher alerts below 5 T (`src/canister.rs:1181`, about two days). One missed alert over a long
weekend is enough.

**Impact.** Nothing is stolen. **C is frozen**, and recovery is risky.

**Fix.** H13: top up automatically; set a 90-day freezing threshold before the handover (the runbook says so; make it a gate);
alert at three levels (5 T warning, 2 T page, frozen = incident); archive finished records so the saved state stays bounded; a
drill that measures the `pre_upgrade` instructions at 10× the expected state.

## T14. The frontend or DNS is compromised

**Path.** An attacker serves a modified `web/` build, through the registrar, the DNS, the host or a dependency. It changes the
`redeem(destination)` argument (a taproot string for wFROG, an address for wAPE), the router a swap uses, or asks for approvals.
The destination is fixed forever at the burn (`src/Wrapper.sol:196`), so a swapped destination is final.

**Likelihood.** Medium. This is the most common way DeFi users lose money that is not a contract bug.

**Impact.** Every redemption and every approval made during the compromise. It is not custody's loss, but it is custody's items.

**Fix.** H14: publish each build's content hash and an IPFS-pinned copy; lock the registrar and enable DNSSEC; set a CSP with no
third-party script origins; before signing, show the redemption destination in full and require the user to type its last 6
characters; for wAPE, default to the user's own address and warn on any other. The watcher alerts when redemptions to
never-before-seen destinations spike (H3's canary). These are `web/` changes, for the web agent.

## T15. The Safe (the contracts' owner) is compromised

**Path.** At or above threshold, the Safe can pause listings, propose new roles, a new `btcVault`, a new treasury, and list new
collections. Every loosening in the registry waits 7 days in public (`CHANGE_DELAY`, `src/ListingRegistry.sol:32`). It cannot change a vault's buyer
or touch the canister (`docs/deploy/key-ceremony.md`). **Not delayed** (2026-09-25, docs/v6/phase1-scope.md §7 item 4): ownership
itself (`Ownable2Step`: `transferOwnership` then `acceptOwnership`, no wait) and `ProtocolTreasury`'s owner setters (`setShares`,
`setOps`, `setBootstrapLoan`, `addCollection`), which redirect the treasury's income and unallocated balance at once.

**Impact.** After 7 public days: a keeper of its choosing (T4 then applies), or a `btcVault` elsewhere. `wrap` then refuses
(`src/Wrapper.sol:148`), but new purchases deliver elsewhere. Held custody is unaffected.

**Fix.** Watcher coverage of every `Propose*` event on 4663 (H5), with the same page-the-guardians urgency as a timelock proposal.

## T16. One EVM key everywhere, and signing oracles on rehearsal installs

**Paths.**
- The EVM address is the same on 4663, 1, 8453 and 46630. It is the vault buyer, every wrapper's custodian, and the holder of
  every NFT. A bug in any one EVM signing path can sign for the others. `evmsend::chain_id_of` checks the chain a transaction
  commits to, but not which *role* is signing.
- `rehearse_sign_evm` (`src/canister.rs:1612`) and `rehearse_sign_sighash` (`:1242`) sign **any** 32 bytes on a canister installed for
  46630 or signet. A signature does not know which chain it will be used on: the same canister's key signs a mainnet EVM
  transaction hash just as well. They are blind oracles by design (`docs/v2/2026-09-22-signet-rehearsal.md`). They are safe only
  while that canister's addresses hold nothing of value **on any chain**.

**Likelihood / impact.** Low if the production canister is a fresh canister id never installed for a rehearsal (its
`chain_id = 4663` disables both). If a rehearsal canister's EVM address ever custodies an NFT on Ethereum, the operator can move it.

**Fix.** A gate: the production canister id has never run a rehearsal build, and every rehearsal canister's addresses are published
as "never send here". H10 (longer term): separate derivation paths per role. For example `b"evm"` for 4663 operations,
`b"nft/<chain>"` for custody on chain *c*, and a different path again for the minter. Then a signing-path bug in one role cannot
move another role's assets. This needs `NftWrapper` to name the holder separately from the minter (a Solidity change) and costs a
migration, so it is ranked low per effort.

---

## What is new in this document

Findings not recorded before, each with its fix:

1. **T6 / H1**: the guardian's pause stops only `tick`. `sign_purchase`, `record_quote`, `nft_set_target`, `nft_sign` and
   `nft_send` ignore it.
2. **T5 / H2**: `nft_set_target`'s cap is the operator's word, unbounded, for any collection. A stolen operator key drains **E_c**
   on every configured chain.
3. **T10 / H3**: deliveries have no rate limit, so a forged 4663 read (or a canister bug) empties **B** as fast as Bitcoin confirms.
4. **T8 / H8**: a delivery that was broadcast, evicted and then refused leaves valid signed bytes in the world, and nothing
   invalidates them before the ticket refunds.
5. **T9 / H9**: outbound decisions use 4663's sequencer-soft pins, not L1-finalized blocks.
6. **T7 / H11**: no wrapper bounds its mint rate on chain.
7. **T4 / H12**: `postCap` has no step bound.
8. **T13**: state grows without bound inside a `pre_upgrade` that must succeed for any upgrade to happen, and an uninstalled buyer
   cannot be recovered by the timelock's `upgrade`.
9. **T1 / H5**: the upgrade *argument* can replace every provider and all of `evm_chains`, and the watcher does not diff it.
