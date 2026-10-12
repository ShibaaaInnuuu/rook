# v9: Runes on Rook (design only)

Status: **design, no code**, 2026-10-10. Nothing was signed or sent. The only network calls were read-only and keyless:
CoinGecko's `coins/markets?category=runes`, and ordinals.com `/rune/<name>` for ids, divisibility and supply.

**Legend.** **[M]** measured on 2026-10-10. **[R]** read from this repository. **[A]** assumption or estimate.

## 0. Summary and recommendation

**Recommendation: after launch, and after the multi-Inscriptions canister change.** Then run a single pilot, **DOG•GO•TO•THE•MOON**,
over a new `Rune` listing kind. Its backing is counted in **lots** (one wX = one lot of a fixed number of runes), so the floor module
keeps its one-unit arithmetic.

Not before launch, for three reasons:
- **The work is large.** It is a new fungible ledger in rook_core's wrapper, a runestone-carrying delivery transaction in the
  canister, a rune buy judge and venue, and a rune price feed. That is ≈ 8-11 weeks with tests and an audit pass (§6).
- **It touches the two riskiest paths.** Custody spending is one: a malformed runestone is a cenotaph that burns every rune in
  its inputs. Price is the other: rune prices are memecoin prices, with no NFT-style floor.
- **The rune market is thin [M].** DOG has a 98 M USD market cap and 0.47 M USD of 24 h volume, most of it on centralised
  exchanges. PUPS has 4.3 M USD and 21 k USD. RSIC has 1.1 M USD and ≈ 0 USD of volume.

**Product fit is the open question.** A rune is itself a memecoin. A Rook coin backed by DOG is a memecoin backed by a memecoin.
That is a weaker story than "backed by a blue-chip NFT", and the operator should decide it before any code.

**What carries over from the Robinhood code (§5):** the runestone codec, the delivered-units evidence and the rune PSBT judge are
kept and tested. The EVM-era rune vaults are not reusable.

## 1. Which runes

ordinals.com `/rune/<name>` and CoinGecko, 2026-10-10:

| rune | id | divisibility | supply | market cap / 24 h volume (USD) | verdict |
|---|---|---|---|---|---|
| **DOG•GO•TO•THE•MOON** | `840000:3` | 5 | 100,000,000,000 (all premined) | 98.2 M / 471 k | **the pilot** |
| PUPS•WORLD•PEACE | `840000:41` | **18** | 1,000,000,000 | 4.3 M / 21 k | later: thin, and amounts need u128 (1e27 raw units) |
| MAGIC•INTERNET•MONEY | `840000:45` | 0 | 21,000,000,000 | 10.5 M / 10 k | no: almost no volume |
| RSIC•GENESIS•RUNE | `840000:28` | 0 | 21,000,000,000 | 1.1 M / ≈ 0 | no: no market |
| UNCOMMON•GOODS, BILLY, LOBO, GAME•OF•BITCOIN… | | | | ≤ 2.7 M / ≤ 35 k | no |

## 2. How a rune-backed collection works

The rest of Rook is unchanged:
- A coin launches on the collection's DBC config, whose claimer is the listing's.
- At graduation, 50 % of the raise becomes backing.
- The backing buys wX from the floor module, and the module's lamports are converted into the asset.

What changes is the **asset**: an amount of a fungible rune instead of a set of distinct items.

**A lot is the unit.**
- The listing fixes `lot` = a whole number of raw rune units. One wX (`ONE` = 10^9 base units) is one lot, and a wX base unit is
  `lot / 10^9` raw units. `lot` must therefore be a multiple of 10^9 / gcd, so rounding never creates runes.
- The lot is sized so one lot costs inside the purchase bounds (1-10 SOL, `frogsol.rs`). For DOG at 0.00098 USD [M] and
  SOL = 109.89 USD, 1 SOL ≈ 112,000 DOG. A lot of 500,000 DOG = 5·10^10 raw units (divisibility 5) ≈ 4.5 SOL, and a wX base unit is
  then 50 raw units.
- Everything the floor module does in units carries over with "item" read as "lot":
  - the buy trigger at one whole unit (`max_item_cost` needs `c >= ONE`);
  - `advance` with `transit_units = ONE`;
  - `items_arrived`;
  - the per-day caps;
  - `depth_items` in `post_floor`.

**Buying.**
- The canister buys **one lot at a time**: one PSBT for at least `lot` raw units, priced at most at the module's cap.
- Marketplaces sell rune UTXOs, each a seller-signed `SIGHASH_SINGLE|ANYONECANPAY` input carrying some amount. Any amount above
  `lot` stays in custody as surplus, counted but not minted.
- When no single listing covers a lot, two or three inputs may be combined. The judge sums them.
- The runes must land on a custody output, either through an edict or as the transaction's default output. The judge decodes the
  runestone with `runestone.rs` and refuses any of these:
  - a cenotaph;
  - a runestone that splits our runes across outputs (`has_spread`);
  - any other rune in our inputs;
  - an inscription on an input.

**Custody.**
- Runes are held in **taproot UTXOs at the canister's custody address**, the same threshold-Schnorr key as inscriptions. The
  custody ledger tracks every UTXO's rune balance (u128 raw units) from two indexers that must agree: ord `/r/utxo` and Ordinals
  Wallet's address balance (`runeindex.rs` `delivered`).
- Rune UTXOs and inscription UTXOs never mix. `custody::frog_at` already refuses an inscription output that also carries runes.
  The fee UTXOs stay rune-free (`custody.rs:289`).

**Wrapping** (rook_core):
- `mint_rune(amount_raw, source_tx, vout)` by the custodian mints `amount_raw / lot · ONE` wX to the module. The remainder stays as
  custody surplus until a later deposit completes a lot.
- Each source outpoint is used once, as `HeldMarker` keys a source today.
- There is no membership proof and no `Item` account: the wrapper keeps `held_raw: u128` and `surplus_raw: u128`.

**Redemption.**
- `redeem(lots)` burns `lots · ONE`. Whole lots only at first: a delivery costs a Bitcoin fee of ≈ 1-5 USD, so fractions are not
  worth delivering.
- It creates a `Ticket { amount_raw: lots · lot, destination: Taproot(addr) }`. There is no drand draw, because a fungible amount
  has no "which item".
- The canister then builds the delivery:
  - inputs: rune UTXOs covering the amount, plus one clean fee UTXO;
  - outputs:
    1. the redeemer's 546-sat output;
    2. `OP_RETURN` runestone: `Edict { id, amount: amount_raw, output: 0 }` with `pointer` set to output 2;
    3. custody's rune change;
    4. custody's sat change.
- `outbound::verify_btc` gains a `RuneDelivery` purpose. It re-decodes the signed transaction's runestone with the canister's
  decoder **and** checks it against an independent decode (ord's `/r/tx` view, or a second implementation), before signing.
- `note_sent` and `record_delivery` subtract `amount_raw`. `refund` re-mints the lots.

**Holder deposits.**
- A holder sends runes to custody in a transaction that also carries the claim marker (`EWD2`, today in an OP_RETURN). A rune
  transfer with a runestone already uses its one OP_RETURN, so the marker must move: to a tagged Solana memo the holder signs,
  naming the outpoint, or to a deposit address derived per claim.
- This is new design work. Holder deposits could stay off for the pilot: a buy-only rune listing, where wX is minted only from
  purchases.

## 3. rook_core changes

- `ListingKind::Rune { id_block: u64, id_tx: u32, divisibility: u8, lot: u128 }`. Every exhaustive match grows a `Rune` arm:
  `keeper_down`, `live_poster`, and the `(Inscriptions, Taproot)` destination checks in `wrap.rs:298`. The custody field stays a
  `bc1p…` (taproot) address, and the poster is the registry keeper, as for Inscriptions.
- **Wrapper.**
  - New fields `held_raw: u128`, `surplus_raw: u128`, `open_raw: u128`.
  - The ledger invariant becomes `supply == (held_raw − open_raw) / lot · ONE + eth_backed`.
  - The `Item`, `ItemSet`, `Pending` and draw machinery is not used for `Rune`. `MAX_HELD` 256 does not apply.
- **Instructions:** `mint_rune` (custodian), `redeem_rune(lots)`, and `resolve` skipped (the ticket is resolved at creation).
  `note_sent`, `record_delivery` and `refund` work on amounts. `Ticket` gains `amount_raw: u128`.
- **Floor module.** No arithmetic change if the unit is the lot. `post_floor`'s price is "lamports per lot". `depth_items` becomes
  "lots of depth within the band" (§4).
- **Limits.** Divisibility ≤ 38 (the rune maximum). `lot` and amounts in u128: PUPS' 18 decimals overflow u64. The wX side stays
  u64, because whole lots fit easily.

## 4. The floor feed for a rune

A rune has no floor in the NFT sense. It has an order book (marketplace PSBT listings, AMM-like pools such as RadFi or Dotswap)
and, for DOG, centralised-exchange books. The posted price per lot is:
- **Book side:** the cost to buy one lot from the cheapest listings on the canister's venue, filling in price order and counting
  only listings whose outputs two ord providers agree carry the rune and amount. `robustFloor`'s spirit carries over: per-listing
  facts, seller clusters, burst exclusion. Its rules are item-based and are rewritten for amounts.
- **Reference:** a volume-weighted mid from at least 2 venues within a band. For DOG these are centralised exchanges where it is
  listed (to verify at design time), and CoinGecko's aggregate as the monitor's D8 reference.
- **Posting:** post the book-side cost only when it is within the band of the reference. Otherwise post nothing, and the module
  goes stale.

Venues:
- Magic Eden's runes API returned 503 during the v2 work (`docs/v2/rates-and-indexer.md:100`) and its ordinals API was down
  all day on 2026-10-10.
- The keeper's `MagicEdenSource.runeUnitPrice` (`keeper/src/sources.ts:52`) was never verified.
- Ordinals Wallet's rune market and UniSat's (keyed) are the candidates. Each needs a read-only spike first, as spike Q5 was
  for Relay.

## 5. What exists in the repository and what can be reused [R]

| code | what it does | status | reuse for `Rune` |
|---|---|---|---|
| `autobuyer/canister-rs/src/runestone.rs` (777 lines, 21 tests) | `RuneId`, `Edict`, `Runestone::decode` (LEB128, delta ids, cenotaph on unknown even tags, non-push opcodes, out-of-range ids or outputs, several runestones), `units_to`, `has_spread`; encoder `Plan { edicts, pointer, mint }` → `script()` | kept through the migration (`ref-robinhood-offchain-deps.md:15`) | **high**: the delivery's runestone and the buy judge's decode |
| `runeindex.rs` (349 lines, 7 tests) | spaced names from the etching, ord `/r/utxo`, OW `/wallet/<addr>/rune-balance`, esplora tx parsers; `delivered(Evidence)` = ord's per-output count corroborated by OW | tested, wired through `oracle.rs` | **high**: custody balances, deposit and buy arrival. Its amounts are u64: widen to u128 |
| `oracle.rs` `name_of`, `delivered_with`, `probe_delivered`, `probe_rune_name` | outcalls; proven on mainnet (`docs/v2/rates-and-indexer.md:175`) | mainnet only (no keyless signet runes indexer) | **medium**: evidence plumbing |
| `psbt.rs` `Quote { rune, units: u128, max_spend_sats, … }`, `Psbt::verify` (24 tests) | the v2 rune buy judge: inputs ours, spend cap, change, `units_to(rune, recipient) >= units`, refuses cenotaphs and spreads | written for UniSat-style rune PSBTs | **high (logic)**: recipient = custody; revalidate against the chosen venue's PSBT shape |
| v2 vault buyer: `looper.rs`, `state.rs`, `policy.rs`, `listing.rs`, `venue.rs`, `relay*.rs`, `bridge*.rs`, `evmtx.rs` (`settleRune`) | the Robinhood-era rune vault: announce, pull, Relay ETH→BTC, buy, settle on 4663 | dead on Solana (`canister.rs:1761`; `solana-migration.md:267` deletes ≈ 1,500 lines) | **no**, except the "record the quote before asking for a PSBT" state-machine pattern |
| `canister.rs:2246-2310` `rehearse_runestone_encode/decode` | query endpoints for rehearsals | kept | yes, for a signet rehearsal of the delivery |
| Solidity `ListingRegistry` `Kind.Rune`, `listRune(Auto)` (divisibility ≤ 38, ref "block:tx"); `AssetVault` / `AutoBuyerVault` `isRune`, cap per whole unit `capWei·qty/10^div`, `settleRune` pro rata; `Distributor` `Mode.ProRata`, dust per output | EVM-era rune vaults | not ported (`rook-core-as-built.md:186`) | prior art only: divisibility, per-whole-unit pricing, pro-rata split, dust minimum |
| `keeper/src/sources.ts` `runeUnitPrice` (ME `runes/market/<name>/info`) | the only "rune floor feed" | unverified | no: replace (§4) |
| `docs/v2/2026-09-22-evm-and-runes-rehearsals.md` | signet: a canister-encoded mint signed by the threshold key (tx `deee13a7…a66c`, block 323,262), credited 1 UNCOMMON•GOODS (`189444:15`) by UniSat's indexer and read back by `Runestone::decode` | done | the template for the delivery rehearsal |

These guards must stay for every non-rune listing, and be relaxed **only** for a `Rune` listing's own outputs:
- `custody.rs:282` (`frog_at`) and `:289-293`;
- `custody.rs:417-425` (`buyer_may_sign` refuses rune inputs);
- `deposits.rs:260`, `:354-358` (`Reason::Runes`);
- `frogbuy.rs:167`, `:230` and `owbuy.rs:190` (no OP_RETURN in a purchase);
- `outbound.rs:255+` (`verify_btc` purposes).

## 6. Effort (≈ 8-11 weeks) [A]

| part | weeks |
|---|---|
| Prerequisite: the canister's multi-Inscriptions change (`ordinals-collections.md` §4) | 1.5-2.5 |
| rook_core: `Rune` kind, the wrapper's fungible ledger, `mint_rune` / `redeem_rune`, amount tickets, tests | 1.5-2 |
| Canister: rune custody ledger (UTXO balances, two indexers, u128), buy judge + one venue, delivery builder with runestone + `RuneDelivery` verification, deposits off for the pilot | 3-4 |
| Rune price feed (book cost per lot + references), keeper/runner hand-in, monitor D8 | 1-1.5 |
| Ceremony, site, docs; a signet rehearsal of buy → mint → redeem → delivery read back by an independent indexer | 1 |
| Audit pass (custody spending and price feed) | 1.5-2 |

## 7. Risks

- **Burning runes.** A cenotaph (an unknown even tag, a malformed edict, a bad pointer) burns every rune in the transaction's
  inputs. Mitigations:
  - the encoder emits only `Body` and `Pointer`;
  - a second decoder checks the signed bytes;
  - inputs carry only the one rune;
  - rune UTXOs are kept small, so one bad transaction loses one UTXO's worth, not the custody.
- **Default-output surprise.** With no edict or pointer covering the change, all unallocated runes go to the first non-OP_RETURN
  output, which is the redeemer's. The pointer to custody's rune change is mandatory, and verification refuses its absence.
- **Indexer disagreement and reorgs.** Rune balances are an indexer's interpretation. Mitigations: two independent indexers must
  agree before anything is minted or considered held, and 6 confirmations.
- **Price.** A memecoin's price can halve in a day. The module's ±20 %/day clamp and staleness rules still apply, and the backing
  of a rune-backed coin is volatile by nature.
- **Liquidity.** Buying a lot may move the book. Lots are kept small relative to visible depth, and the module's `fill_band_bps`
  caps the premium.
- **Amounts.** u128 throughout the Bitcoin side and the wrapper ledger (PUPS: 10^27 raw units).
- **Venue PSBT shapes change** without notice (v2's experience). One judge per venue, each pinned by fixtures.

## 8. Decision asked of the operator

1. Is a memecoin-backed-by-a-rune product wanted at all (§0)?
2. If yes: DOG only, after the multi-Inscriptions change ships and the five new Ordinals collections are awake. Buy-only (no holder
   deposits) for the pilot.
3. The lot size (DOG: 500,000 DOG ≈ 4.5 SOL on 2026-10-10) and the venue spike (OW runes market vs UniSat).
