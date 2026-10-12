# Solana NFT collections in the canister-only design: membership, custody, buys, delivery, floor

Written 2026-10-10 on branch `golive-2026-10-08` (HEAD `bd2c93c`). This is the canister side of `solana-nfts-design.md` under
`canister-only-solana.md` (D-l). In that design there is no rook_core, and the canister keeps the books. The on-chain checks of
`solana-nfts-design.md` §2 and §4 become checks the canister runs itself on accounts it reads at `finalized`, with two RPC
providers that must agree.

**What exists.** The pieces below are standalone modules with clear interfaces; since 2026-10-11 they are wired into the books home
by `src/solnftflow/` (§9, "Built").

**Market data** below is from Magic Eden's public API, read 2026-10-10 around 10:00–14:40 UTC. Accounts are read at mainnet
`finalized`, slots 455,291,570–455,298,137. Everything is read-only: nothing was signed or sent on mainnet.

---

## 0. Findings

1. **Tensor Marketplace (TComp) is the only venue the canister can buy from without a key, today.** Its listings escrow the item
   at a `ListState` PDA. The taker does not need a co-signature unless the listing set one, and none of the listings read had.
   - `buy_legacy` (legacy and pNFT) and `buy_core` are built, judged and executed in LiteSVM against the program dumped from
     mainnet.
   - In every case the NFT lands in the stand-in canister wallet, which pays exactly what the ask computed (§3.3).
2. **Magic Eden is not keyless.**
   - **mmm pools:** every fulfill instruction requires the pool's `cosigner` to sign, and that key must differ from the owner. All
     100 Mad Lads pools and every pool read for other collections name Magic Eden's notary `NTYeYJ1w…` as cosigner.
   - **M2:** the auction house `E8cU1WiR…` has `requires_notary = 1`.
   - Magic Eden's instruction API, which returns co-signed transactions, answers **401 without an API key**.
   - mmm is built and judged anyway, so a co-signed transaction can be judged the same way. It was executed in LiteSVM with the
     recorded pool's cosigner field replaced by a stand-in key.
3. **The cheapest asks are almost all M2.** Keyless Tensor asks sit above the floor (table in §3.1): +1 % (SMB Gen2), +13 %
   (Pythenians), +19–29 % (Mad Lads, DeGods, Galactic Geckos, Okay Bears), up to +209 % (Famous Fox Federation). Some collections
   have no Tensor listing among the cheapest 100 at all (Taiyo Robotics). With the Ethereum side's `max_premium_bps`-style bound
   of about 10 %, **keyless buys work today only for SMB Gen2, and for Claynosaurz once the Tensor AMM path is built** (§9).
   Buying at the floor needs either a Magic Eden API key (co-signed M2 and mmm transactions, judged by the same judge) or the M2
   observation in item 4.
4. **M2 does not always demand the notary.**
   - Of 168 recent M2 buys on Magic Eden's own feed, 63 were signed by the buyer alone; the notary was passed but did not sign.
     Example: `2TAeN4kwJ4VKUyYtGDG8zct4wL5yL6xzcTC2Yi9rBgxc1gMGJW23eYiTDGPrg2NKYzLXqRX6pXpLCResdreftvNZ` (Mad Lads, one
     signature), versus `29AYiCYHkV6bWcVCZwouDHoS4d7ftAmJD16qA2dmvKpLSpCx5vURqGn4M1bLF9V24D66XGAPUMv7fzMTLVDbKApv`
     (notary-signed). Both are in `autobuyer/canister-rs/tests/fixtures/solnft/opensea/`.
   - The auction house's `nprob = 79` byte suggests the notary check is probabilistic.
   - M2 is closed-source, though its interface is published (`programs/m2_interface/m2.json` in `me-foundation/mmm`). A keyless M2
     path is the most valuable follow-up spike (§9, S-N1).
5. **OpenSea is not a separate venue.**
   - Solana fills go through `POST /api/v2/listings/fulfillment/actions`, which needs an API key (401 without one; the free
     instant keys expire after 7 days).
   - Every observed fill ran on M2, mmm, TComp or TAMM, plus a top-level 1 % fee transfer to `HoTh5YuQ7wQDStcM7BSGx8kttCHHzdGSiBy8vz1Fic6Z`.
     An example is TComp `buy_legacy` in `3S6bmZ9Fcp9VdDVmZFJrhkdm8b1yXT6euDv7TqFbeTEZeCV9ejXN6vi4UErbgRz4njxZBRfUWYwWoTev4Uatcq8c`.
   - So it re-routes the same listings at about 1 % more. A native OpenSea program, `7LFyQ8RC…`, a TComp fork with 45
     transactions from 3 wallets, may be in testing.
   - The judge (§3.4) refuses such a transaction as built: it carries an extra instruction (J1), and OpenSea is the taker broker
     (J2).
   - Double counting is avoided by keying asks on the listing account, which OpenSea exposes as `svm_order.order_state`, and on
     the asset (§5).
   - Recommendation: not a venue. At most a fallback route, if a key is ever configured.
6. **All 12 collections are Token Metadata:** ten pNFT and two legacy (FFF, TFF), with no Core and no cNFT.
   - Their rule sets are the Metaplex Foundation one (latest revision `Pass`) or the Compatibility one (`AdH2Utn6…`, a single
     `Pass` revision; SMB Gen3 and Cyber Frogs). Some Cyber Frogs items have no rule set at all.
   - Wallet-to-wallet delivery passes under both rule sets in LiteSVM.

## 1. Modules and what the later flows call

All are in `autobuyer/canister-rs/src/`, and all are pure except the `async` reads. `lib.rs` gets four `pub mod` lines. A single
line with `#[path]` sub-modules is refused by the H3 guard (`tests/audit_nft_r7.rs`, E7-I-1).

| Module | Contents |
|---|---|
| `solnft.rs` | Layouts: SPL mint and token account, Token Metadata `MetadataV1` / editions / `TokenRecord`, mpl-core `AssetV1` / `CollectionV1` with plugins, external adapters and `Royalties`. `is_member`, `in_custody`, `read`, `check` |
| `solnftbuy.rs` | Venue layouts: TComp `ListState`, mmm `Pool` / `SellState`, M2 `SellerTradeStateV2` and auction house. `verify_ask`, `verified_asks`, `best_ask`, the builders (`tensor_buy`, `mmm_fulfill_sell`, `buy_instructions`), `judge` |
| `solnftsend.rs` | `send_instructions` (legacy / pNFT / Core) and `judge_send` |
| `solnftfloor.rs` | `floor_from`, `read_floor`, `candidates_from_me`, `reference_from_me_stats` |

The flow a later `solnftflow` should run, step by step:

1. **Floor**, every pass:
   - fetch Magic Eden's `/v2/collections/<symbol>/listings?listingAggMode=true` and `/stats` (§5);
   - turn them into candidates with `solnftfloor::candidates_from_me` and a reference with `reference_from_me_stats`;
   - `read_floor(providers, candidates, collection, standard, now, reference, params)` returns a `SolFloor`, which goes to the
     books' floor module (as the Ethereum feed's `FloorPost` does).
2. **Buy**, when the module's buy signal fires:
   - take the same pass's `(Ask, Member)` list and pick with `solnftbuy::best_ask(asks, limit, cosigned)`, where `limit` is the
     books' floor bound (floor × (1 + premium), at most the advance) and `cosigned` is empty at launch;
   - build with `buy_instructions(canister, ask, member)`;
   - **judge** with `judge(ixs, &Expect { payer, ask, member, collection, limit })`;
   - sign and send on a send line (the Ethereum path judges a Seaport order the same way);
   - after it lands, `solnft::check(providers, asset, collection, standard, canister)` at `finalized`, then book `held += 1`.
3. **Deliver** a resolved ticket:
   - `solnft::check` for the canister;
   - `send_instructions(canister, destination, member)`, then `judge_send`, sign and send;
   - after it lands, `solnft::check(…, destination)` decides the delivery, then the book records it.
4. **A transaction from an API** (a co-signed Magic Eden one, if a key is ever configured): decompile it with
   `soltx::Parts::parse` and `decompile`, then pass the instructions to the same `judge`. Anything not canonical is refused.

## 2. Membership and custody (`solnft.rs`)

`is_member(asset, collection, listing_standard, accounts)` returns `Member { asset, standard, collection, royalty_bps,
creators, rule_set, core_owner }` or an `NftError`.

**Token Metadata** (legacy and pNFT; a Token Metadata listing accepts both kinds of member) requires all of:
- the mint is classic SPL Token (Token-2022 refused), initialized, supply 1, decimals 0;
- the metadata is `MetadataV1` of this mint, with standard `None`, `NonFungible` or `ProgrammableNonFungible`;
- `collection == Some { verified: true, key }`, and the metadata is not itself the collection NFT;
- the edition is the mint's PDA (derived with `edition_nonce` too) and a master edition. Prints and fungibles are refused.

**Core** requires an `AssetV1` (`HashedAssetV1`, compressed, is refused), `update_authority == Collection(key)` and a
`CollectionV1`. Refused (`Unsupported`):
- any permanent delegate (freeze, transfer, burn, freeze-execute) on the asset or the collection;
- an owner-managed delegate plugin on the asset whose authority is neither `Owner` nor the owner's address;
- an external adapter `LifecycleHook`, `Oracle` or `LinkedLifecycleHook`, or any adapter with lifecycle checks;
- a `Royalties` program allow-list without the System Program.

**`in_custody(member, holder, accounts)`:**
- Token Metadata: the holder's ATA is SPL Token, of this mint, owned by the holder, amount 1, with no delegate and no close
  authority. A legacy account must not be frozen. A pNFT's token record must be `Unlocked`, with no delegate and no locked
  transfer; its token account is always frozen, which is fine.
- Core: the asset's owner is the holder and the asset is not frozen.

**Reads.** `read(providers, keys)` is one agreed `getMultipleAccounts` at `finalized`. It refuses with fewer than two providers,
when any provider fails, and when the providers disagree. `check(providers, asset, collection, standard, holder)` reads
`member_keys` and `custody_keys` once and decides both.

**Missing accounts.** A compressed NFT (cNFT) has no account at all, so it fails as `Missing`. A collection's members are never
cNFTs here: `deployments/launch/solana-collections.json` flags `compressed: false` for all 12, and the check script verifies
sample members on chain.

## 3. Buying (`solnftbuy.rs`)

### 3.1 Venues

| Venue | Program | Standards built | Keyless? | Verified in LiteSVM |
|---|---|---|---|---|
| Tensor Marketplace (TComp) listing | `TCMPhJdwDryooaGtiocG1u3xcYbRpiJzb283XfCZsDp` | legacy, pNFT (`buy_legacy`), Core (`buy_core`) | **yes**, unless `ListState.cosigner` is set | yes: Famous Fox (legacy), Mad Lads (pNFT, Foundation rule set), OTC Desks (Core) |
| Magic Eden mmm pool | `mmm3XBJg5gk8XJxEKBvdgptZz6SgK4tXvn36sodowMc` | legacy (`sol_fulfill_sell`), pNFT (`sol_mip1_fulfill_sell`) | **no**: the pool's cosigner (Magic Eden's notary) signs | yes, with a stand-in cosigner: TFF (legacy), SMB Gen3 (pNFT, Compatibility rule set). Without the cosigner it cannot be signed |
| Magic Eden M2 | `M2mx93ekt1fmXSVkTrUL9xVFHkmME8HTUi5Cyc5aF7K` | read only (floor) | no (notary; see §0.4) | n/a |
| Tensor AMM (TAMM) pool | `TAMM6ub33ij1mbetoMyVBLeKY5iP41i4UPUJQGkhfsg` | not built | yes (pool cosigner unset) | program dumped; see §9 |
| OpenSea | (routes the above) | not a venue | no (API key) | n/a |

Cheapest ask per venue among Magic Eden's 100 cheapest aggregated listings, in SOL and above the floor:

| Collection | Floor (M2) | Tensor listing | Tensor AMM | mmm |
|---|---|---|---|---|
| Mad Lads | 8.352 | 10.000 (+20 %) | — | — |
| SMB Gen2 | 12.659 | 12.750 (+1 %) | — | — |
| Claynosaurz | 6.979 | — | 7.472 (+7 %) | — |
| Famous Fox Federation | 3.238 | 10.000 (+209 %) | — | — |
| DeGods | 3.483 | 4.150 (+19 %) | — | — |
| Galactic Geckos | 4.082 | 5.250 (+29 %) | — | — |
| Taiyo Robotics | 14.948 | — | — | — |
| Okay Bears | 1.238 | 1.583 (+28 %) | — | — |
| SMB Gen3 | 2.338 | 5.000 (+114 %) | — | 8.221 (+252 %) |
| Transdimensional Fox Federation | 1.208 | 2.100 (+74 %) | — | 1.641 (+36 %) |
| Pythenians | 1.200 | 1.350 (+13 %) | — | 1.929 (+61 %) |
| Cyber Frogs | 1.323 | 3.000 (+127 %) | — | — |

### 3.2 Asks

**Discovery.** An off-chain index names the candidates (`Candidate { venue, listing, asset }`). Only the addresses are used.

**`verify_ask` checks, on the accounts read at `finalized`:**
- the listing account's program and layout;
- that it is this asset's PDA;
- that it is live: version, expiry, SOL currency, no private taker, the pool not expired and still selling;
- that the escrow really holds the item:
  - Tensor: the `ListState`'s ATA, or the Core asset's owner;
  - mmm: the pool's ATA;
  - M2: its program-as-signer `["m2","signer"]` = `1BWutmTv…`, or the seller with that PDA as delegate.

**`Ask.total`** is everything the buyer pays for the item:
- Tensor: `amount` + 2 % taker fee (tensor-toolbox `TAKER_FEE_BPS`) + royalties. A pNFT's and Core's royalties are enforced at
  100 %; for legacy the canister passes `optional_royalty_pct = None` and pays none.
- mmm: the one-step-over-spot price + LP fee + the pNFT royalty, with maker and taker fees at 0.

**`Ask.rent(member)`** is the buyer's new accounts: the ATA (0.00203928 SOL) and, for a pNFT, the token record (0.00144768
SOL).

**`best_ask`** picks the cheapest `total + rent ≤ limit` among keyless asks, and among co-signed venues only if configured. It
never picks M2. Ties go to the lower price, then the lower listing address (deterministic across replicas).

### 3.3 Measured in LiteSVM (`autobuyer/solnft-svm`, real mainnet accounts, the programs dumped from mainnet)

| Purchase | Price | `Ask.total` | Paid by the canister | CU |
|---|---|---|---|---|
| Tensor, legacy (FFF) | 10 SOL | 10.2 | total + ATA rent + 5,000 fee, exactly | 106,250 |
| Tensor, pNFT (Mad Lads) | 10 SOL | 10.62 | total + rent + fee − 0.00105664 (the escrow token record's lamports, refunded to the payer by Token Metadata), exactly | 264,520 |
| Tensor, Core (OTC Desks) | 34.575 SOL | 36.99525 | total + fee, exactly | 50,122 |
| mmm, legacy (TFF), stand-in cosigner | 1.640989048 | same (LP fee 0, no royalty) | total + rent + 2 signature fees, exactly | 117,648 |
| mmm, pNFT (SMB Gen3), stand-in cosigner | 8.13942546 | 8.546396733 (LP 1 %, royalty 5 %) | total + rent + 2 fees − refund, exactly | 312,306 |

`BUY_CU` is 400,000. Every transaction is the canister's own legacy message (`soltx::compile_parts`), signed over those bytes
and assembled by `soltx::assemble`. The largest is under the 1,232-byte limit (`wire_size_fits`).

### 3.4 The judge

`judge(ixs, Expect { payer, ask, member, collection, limit })` refuses unless all four rules hold:

- **J1:** only Compute Budget and the ask's venue program appear, with exactly one venue instruction. An ATA creation, a System
  transfer or a second purchase is refused.
- **J2:** the venue instruction is the canonical one for this ask. The discriminator must match the asset's standard, and
  **every account and its flags** must equal `buy_instructions`'. So:
  - the destination is the canister's own ATA (or, Core, the canister);
  - the fee vault is Tensor's shard PDA;
  - there is no taker broker (the sentinel is the program id);
  - the seller, maker broker and rent payer are the listing's;
  - the creators are the metadata's, in order;
  - the rule set is the item's.
- **J3:** the signed maximum (`max_amount`, `max_payment_amount`) is at most the ask's, so a front-run cannot raise it. The most
  the instruction can spend (the maximum plus Tensor's fee on the listed amount), plus rent, is at most `limit`. mmm fees stay
  within the program's 5 %. A legacy item carries no buyside royalty, and there is no trailing data.
- **J4:** the member is this asset, of this collection, decided on the same read.

Tested refusals:
- a stolen destination (the buyer and its token account replaced);
- a taker broker slipped in;
- another rule set;
- a `buy_core` discriminator on a pNFT;
- a raised maximum;
- trailing data;
- a limit one lamport short;
- another collection;
- mmm fees over 5 %;
- a legacy buyside royalty.

A maximum under the listing passes the judge, and the program refuses it on chain.

## 4. Delivery (`solnftsend.rs`)

Every delivery is wallet to wallet. `send_instructions(from, to, member)` produces:

- **legacy:** ATA `CreateIdempotent` for the destination, SPL `TransferChecked(1, 0)`, then `CloseAccount` of the canister's
  emptied ATA. Its rent comes back.
- **pNFT:** Token Metadata `TransferV1`. It takes both token records, the item's rule set and Token Auth Rules; Token Metadata
  creates the destination ATA and token record, and the canister pays for them.
- **Core:** mpl-core `TransferV1` with the collection.

A destination that is empty, the canister itself, or off the ed25519 curve (a PDA) is refused. `judge_send` accepts only that
exact instruction list.

Measured in LiteSVM after each purchase above; the redeemer holds the item afterwards (`in_custody`), the canister no longer:

| Delivery | CU |
|---|---|
| Legacy | 29,602–31,102 |
| pNFT, Foundation rule set | 138,359 |
| pNFT, Compatibility rule set | 111,620 |
| Core | 7,284 |

`SEND_CU` is 250,000.

## 5. Floor (`solnftfloor.rs`)

The floor follows the Ethereum and Ordinals feeds:

1. **Candidates** come from Magic Eden's public listings endpoint (`listingAggMode=true` aggregates M2, mmm and TComp; no key
   needed). The list is sorted and deduplicated.
2. **Verification** happens on chain at `finalized`, in two agreed reads: the membership and the listing (`verified_asks`). M2
   asks count here: a notary-gated listing is still a real market price.
3. **One ask per asset**, the cheapest, so an aggregator re-listing the same escrow counts once. Then **one per seller**.
4. The floor is the `rank`-th cheapest of at least `min_sellers` sellers. Defaults: 3 sellers, rank 2, as `ethnft::FloorParams`.
5. **Refusal on disagreement:** the verified floor and Magic Eden's stats floor (`floorPrice`) must agree within `max_gap_bps`
   (2,000). A provider disagreeing on any account refuses the whole read.
6. `SolFloor` also reports `depth` (sellers within 5 %), the cheapest ask, and the cheapest **keyless** all-in total with its
   venue. That tells the flow whether a buy is possible at all under its premium.

**Real case** (`floor_mad_lads` fixture, slot 455,298,137):
- 9 aggregated listings, all M2, all verified on chain. 9YCN4w… lists three, so they count as one seller.
- Floor 8.353 SOL (the cheapest 8.352), against Magic Eden's 8.352.
- No keyless ask among them.

**How the canister reaches Magic Eden** is the flow's choice, either:
- an IC HTTPS outcall whose transform keeps only `(listingSource, pdaAddress, tokenMint)`, which `candidates_from_me` needs
  (replicas then agree), plus `floorPrice`; or
- the runner posting the two answers.

Either way they are hints: every fact comes from the chain, and the reference only vetoes.

## 6. Collections

`deployments/launch/solana-collections.json` holds, per collection: key, name, symbol, collection mint, standard, rule set(s),
supply, Magic Eden symbol, Tensor slug (all confirmed), X and website (verified against the official sites), floor, 30-day sales,
tier and `T_wX`, sample members verified on chain, and the excluded candidates. `scripts/check-solana-collections.py` checks it
offline, on chain, and with `--market` for floor drift. The unlock order is in `canister-only-solana.md` §16.

Every collection lands on the 85 SOL tier. `T_wX = max(3, ⌈85 / floor⌉)`:

| Collection | Symbol | Standard | Floor | `T_wX` |
|---|---|---|---|---|
| Mad Lads | wMADLAD | pNFT | 8.35 | 11 |
| SMB Gen2 | wSMBGEN2 | pNFT | 12.66 | 7 |
| Claynosaurz | wCLAYNO | pNFT | 6.98 | 13 |
| Famous Fox Federation | wFFF | legacy | 3.24 | 27 |
| DeGods | wDEGOD | pNFT | 3.48 | 25 |
| Galactic Geckos | wGECKO | pNFT | 4.08 | 21 |
| Taiyo Robotics | wTAIYO | pNFT | 14.95 | 6 |
| Okay Bears | wOKAYBEAR | pNFT | 1.24 | 69 |
| SMB Gen3 | wSMBGEN3 | pNFT | 2.34 | 37 |
| Transdimensional Fox Federation | wTFF | legacy | 1.21 | 71 |
| Pythenians | wPYTHENIAN | pNFT | 1.20 | 71 |
| Cyber Frogs | wCYBERFROG | pNFT | 1.32 | 65 |

## 7. Tests and fixtures

- **`autobuyer/canister-rs/tests/solnft.rs`** (16 tests): decoders on real bytes; members of every standard; non-members
  (collection, unverified, print, Token-2022, supply, unread); real refusals:
  - Retardio Cousins Classic's collection `PermanentFreezeDelegate`;
  - a Critters asset whose `TransferDelegate` is handed to the update authority;
  - USDC;
  - a Tensorians cNFT (no account);
  - a `HashedAssetV1`.

  It also covers custody (escrows, M2's program-as-signer, delegates, locks, frozen) and the two-provider read (agreement,
  disagreement, one provider). Then asks and their totals; stale and foreign asks; `best_ask`; the judge on Tensor and mmm;
  wire sizes; delivery and its judge; and the floor on real listings and its rules.
- **`autobuyer/solnft-svm`** (standalone crate, own `[workspace]`, LiteSVM 0.10; 7 tests): every purchase in §3.3 and delivery
  in §4. Also: a purchase without the mmm cosigner cannot be signed, and a tampered purchase fails the judge, while a maximum
  under the listing fails on chain.
  - Program bytes: `solana/tests/fixtures/{mpl_token_metadata,mpl_core,mpl_token_auth_rules}.so` (sha256 equal to mainnet's
    today) and `fixtures/programs/{tensor_marketplace,mmm}.so`.
  - The two marketplace programs were dumped from mainnet on 2026-10-10 and cut to their ELF extent: `solana program dump`
    pads to the program data length, and cutting trailing zeros below the ELF extent breaks the loader. Their sha256 are in
    `fixtures/programs/SHA256SUMS.dumped`.
  - Run with `cd autobuyer/solnft-svm && cargo test`.
- **Fixtures:** `autobuyer/canister-rs/tests/fixtures/solnft/*.json` hold real accounts, one `getMultipleAccounts` per scenario,
  so every account of a scenario is from one slot. They were recorded by `autobuyer/solnft-svm/record_fixtures.py` (read-only,
  public RPC and Magic Eden API), with the clock set to the slot's block time. `opensea/` holds the OpenSea API spec extract and
  three real transactions: the aggregator's TComp buy with the 1 % fee, and an M2 buy with and without the notary's signature.

## 8. Risks that stay

- **Rule sets.** Both the Foundation and the Compatibility rule set are `Pass` today. Their owner can publish a stricter
  revision. Delivery is wallet to wallet, which every Metaplex rule set has allowed, and the buy path's transfer is done by the
  marketplace program.
- **Royalty and fee constants are the programs' own today.** Tensor's 2 % comes from tensor-toolbox 0.8.1, and mmm's fee caps
  from its source. A program upgrade that raises a fee:
  - fails the purchase on chain where the maximum binds: the price + royalty in Tensor's `max_amount`, everything in mmm's
    `max_payment_amount`;
  - is caught by Tensor's taker fee bound only through `limit`. The fee is computed on chain from the listing amount, so a higher
    fee can spend up to (new − 2 %) × price more than the judge assumed.

  The LiteSVM tests pin today's behaviour to the lamport. Re-dump and re-run them before each wave of buys.
- **Tensor listings above the floor** (§0.3). With keyless buying only, most collections will not fill under a tight premium.
  This is an operator decision (O-T1 below).
- **The token record refund.** A pNFT purchase refunds the escrow's token record lamports to the payer, so the canister spends
  slightly less than `total + rent`. The books should book the measured outflow, not the ask.

## 9. Built (2026-10-11) and not built

**Built: `autobuyer/canister-rs/src/solnftflow/`** (`mod.rs`, `floor.rs`, `buy.rs`, `deliver.rs`, `run.rs`, `api.rs`), the flow of §1
on the books home, for every collection listed as `ListingKind::SolNft { collection, custodian = K_main }`:

- **Floor.** The runner hands in Magic Eden's answers: `sol_nft_evidence(coll, listings_json, stats_json)` (operator-only, refused while
  paused or mid-tick; at most 64 KiB of listings and 4 KiB of stats; the cheapest 40 listings kept, by Magic Eden's price; a newer
  hand-in replaces the older). Only `(listingSource, pdaAddress, tokenMint)` and `floorPrice` are kept. Each hand-in is read once, at
  finalized through the agreed providers (`solnftbuy::verify_all` on one read of `ask_keys_all` + `ask_keys_then`); a hand-in older than
  30 min is not read. `floor_from` (3 sellers, rank 2, 20 % gap to the reference) is posted to the books (`post_floor`, clamped into the
  module's bounds: 20 % a day; confidence `10,000 − 2 × gap bps`; depth = sellers within 5 %); the books close the mint and the buy once
  it is 3 h old.
- **Buy.** When the module's signal fires (`books_sink_view` + `itemsink::may_start`, as `home_books`), the ask is `best_ask` over the same
  read's asks (kept 120 s), with the limit `min(floor × (1 + premium), max_item_cost)`; M2 buyer-only (`solnftm2`), **on by default
  since 2026-10-12** (below). Every held asset is skipped. The custody cap of `sol:<collection>` (the install's, else stage 0:
  3 items, 10 ETH, at the SOL/ETH rate) is checked first. The purchase is judged, then the advance is a chain link (`OpKind::Advance`,
  `Scope::Module`, `Effects { lamports_out }`, memo `rook:v1:<coll>:advance:<id>`), then ONE transaction on the money line signed by
  `K_main` and `K_coll[i]`: `SetComputeUnitLimit`, `M[i] → K_main` of the ask's all-in (`total + rent`), the purchase, the memo, judged
  whole (`buy::judge_tx`: exactly that transfer and that memo beside the venue judge, which admits no `SetComputeUnitPrice`, B2-01) and
  gated by `pre_sign`. Failed or never landed: the link is undone (one transaction: nothing left `M[i]`). Landed: `solnft::check` for
  `K_main` at finalized, then `mint_item` (key = the mint, source = the landed signature's first 32 bytes) and `settle_advance` at the
  advance, together, through `desk::book_outcome` (B1-06; `OwedWrite::SolNftArrived`). Not confirmed after 5 reads: an incident.
- **Deliver.** A resolved ticket to `Destination::Sol(d)`: `solnft::check` for `K_main`, `send_instructions` + `judge_send`, signed by
  `K_main` alone (no link and no memo: no supply or backing changes). Landed: `solnft::check` of `d` decides `record_delivery`, else
  `note_sent` (re-checked each pass, recorded once the read agrees). A destination that is not a wallet is marked undeliverable at once;
  a refusal by simulation or on chain (a rule set veto, B2-09) three times marks it too. `Destination::Burn` stays refused by the ledger.
- **One step at a time.** `LineOwner::SolNft` / `LineStep::SolNft(SolNftStep::{Buy, Deliver, Table})` on the money line, deliveries before
  purchases, at most 24 signed a day, never while the collection has an open link (`collection_busy`). The state rides the upgrade in
  `SolLines.solnft_state`. Wired into the runner's `tick` and the canister's timer (after the pool desk, before the cranks).
- **Surface.** `sol_nft_evidence`; `sol_nft_configure(coll, standard?, premium_bps?, m2?)` (operator; the standard only while nothing is
  held; a premium raise or M2 switched on waits 24 h, a lowering or M2 switched off applies at once; premium at most 3,000 bps,
  **default 1,000 bps: O-T1**; **M2 on by default** for a collection the flow holds no state for yet, while a collection whose state was
  saved before keeps its value); `sol_nft_set_off(off)` (stop-only, on the pause test's exempt list); `sol_nft_status(coll)` (knobs, the last asks and
  floor, items in custody with their signatures, what is out, the M2 lookup table). `books_view` shows a SolNft item's key as the mint in base58.
- **Tests.** `tests/solnft_flow.rs` (9, the fake chain with Mad Lads' mainnet accounts; the fake executes Tensor's purchase and Token
  Metadata's transfer as the programs leave the accounts): the floor posted from the 9 real M2 listings (8.353 SOL); stale floor and
  old hand-ins; the Tensor purchase end to end (the link, one transaction with two signatures and the memo, custody, `held = 1`, the
  supply the ledger's); a failed landing undone; a delivery recorded; a PDA destination and a vetoing rule set marked undeliverable,
  the burn refused; the custody cap; the judge's refusals (a compute-unit price, a second transfer, an advance off the ask, a hostile
  candidate, an ask over the limit) with nothing signed. `solnft-svm/tests/flow_tx.rs`: the same one transaction on Tensor's mainnet
  bytes (the advance pays the purchase exactly), then the delivery.

**M2 buyer-only on by default (2026-10-12, the operator's decision; audit C1-03 fixed further).** Magic Eden's cheap asks of pNFT
collections (Mad Lads) are M2 listings, and an M2 pNFT purchase as the flow sends it (the nonce's advance, the advance with its second
signature, `deposit` + `buy_v2` + `mip1_execute_sale_v2` with ~30 accounts, the memo) is 1,395–1,461 bytes as a legacy message, over
Solana's 1,232: the C1-03 fix skipped it. It is now the same ONE transaction (same instructions, same judge, same two signers, same
link and undo) compiled as a **v0 message** that loads M2's fixed accounts from the canister's own **address lookup table**
(`solnftflow/table.rs`): the auction house and its notary, authority and treasury, M2's signer PDA, `K_main`'s M2 escrow, the Token,
Token Metadata, Auth Rules and ATA programs, the Instructions and Rent sysvars, Metaplex's two common rule sets; nothing of a seller or
an item. One table for the canister, created and filled in one transaction by `K_main` alone (`SolNftStep::Table`, ≈ 0.0044 SOL of
rent) the first time an M2 ask waits for it; read at finalized before every use (the ALT program's, `K_main` its authority, active,
exactly what it was made with, its last extension older than the finalized slot); never extended, deactivated or closed by the canister.
`solsend::compile_with` refuses a v0 message that does not decompile, against the table as read, to the judged instructions; the line
keeps the table for a fallback re-sign (`Saved.tables`). Measured (`buy::composed_size`, an upper bound with the longest memo): every
M2 pNFT fixture 1,028–1,094 bytes (Mad Lads 1,061; 1,042 as signed), the legacy M2 one 980; Tensor's stay legacy messages (pNFT 1,153,
legacy 961, Core 788), MMM legacy 1,127. Two transactions (the advance first, then the purchase signed by `K_main` alone) was measured
and rejected: the purchase alone with its nonce and memo is still 1,282–1,348 bytes. Not covered: an MMM pNFT purchase (1,356 bytes as a
legacy message) is still skipped before anything is booked. Tests: `tests/audit_c1_solnft.rs` (`fixed_further_c1_03_…`: Mad Lads
bought from M2 end to end, a failed M2 purchase undone with the books as before, a table not as made never used, M2 on by default / off
at once / on after 24 h) and `solnft-svm/tests/m2_table.rs` (the table created, read back, and the flow's v0 transaction landing on
M2's mainnet bytes: the advance pays the purchase, the item in `K_main`'s custody).

**What needs a live chain.** The flow has run on a fake chain and the transaction on LiteSVM only: the agreed reads of real providers
(response sizes of 40 candidates), the money line's durable nonce with two signers, the timer's cadence, and whether keyless Tensor asks
ever sit within the books' cap (`max_item_cost` is the floor plus about the mint spread: SMB Gen2's +1 % fits, Mad Lads' +20 % does
not; §0.3). The pNFT's token-record refund (≈ 0.001 SOL) stays at `K_main` as fee float (the books settle at the advance).

**Not built:**

- **S-N1: M2 without the notary** (§0.4). Decode `buyV2` and `mip1ExecuteSaleV2` from `m2.json`, replay the 63 buyer-only
  transactions in LiteSVM with M2's dumped bytes, and learn when the notary is skipped. If it is deterministic (a slot or hash
  rule over `nprob`), the cheapest asks become keyless.
- **Tensor AMM (TAMM) `buy_nft` / `buy_nft_core`.** Keyless when the pool's cosigner is unset; Claynosaurz is at +7 %. Needs the
  `Pool` layout, the whitelist account and the linear and exponential price (spl-math `PreciseNumber` rounding). The program is
  dumped in the scratch, not in the repo.
- **A Magic Eden API key** (operator decision O-T2). It would unlock co-signed M2 and mmm transactions, judged by `judge` as they
  are. Magic Eden's API returns the transaction; the canister only adds its signature, inside the blockhash's life.
- **mmm Core pools** (`sol_mpl_core_fulfill_sell`), legacy TSWAP pools, Token-2022 and WNS: not needed by the 12.
- **Operator decisions:**
  - O-T1: the premium a keyless Tensor buy may pay over the floor (now `sol_nft_configure`, default 10 %; the books' item cap binds
    first unless the collection's `conv_bps` is raised);
  - O-T2: whether to hold a Magic Eden API key, which reintroduces a third party into the buy path and is not keyless;
  - O-T3: OpenSea, recommended no (§0.5).
