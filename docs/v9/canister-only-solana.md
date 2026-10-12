# Canister-only Rook on Solana (D-l): design

Written 2026-10-10 on branch `golive-2026-10-08` (HEAD `9c43c33`); revised the same day after spikes S1–S3 (below). **This is a
design. Nothing has been signed or sent, and nothing is deployed.** It is the document that `pause-2026-10-10.md` §5 asks for.
Code so far: the W2 instruction builders and decoders (`autobuyer/canister-rs/src/solix2`, §2), not wired into any flow.

**Legend.** **[R]** read in this repository (path given). **[M]** measured earlier and recorded in a doc cited here. **[W]** from
Meteora, Jupiter or IC documentation and not checked here; each one is listed as a spike in §15. **[A]** an assumption or
estimate made for this document.

---

## Revision after spikes S1–S3 (2026-10-10)

Spikes S1–S3 ran on 2026-10-10 (`docs/v9/spikes/s1-s3-canister-only.md`, "the spikes" below). Nothing failed outright: DBC and
DAMM v2 work with a wX quote and a canister-key claimer, every fee arrives in wX, and multi-key signing from the canister works.
But the wX/SOL pool sizing was wrong by ≈ 20×, Jupiter's routing is time-limited, and a few bounds are new. The sections below are
corrected in place; this list says what changed and where.

1. **wX/SOL pools: none, or tiny ones, at launch; fees build them (§5.1, §5.3, §12, O-1).** Measured: at `[p/4, 4p]` a buy of
   Δ SOL moves the pool price by ≈ Δ / (SOL a side), so the old seeds (0.5 SOL a side) gave +10.5 % on a 0.1 SOL buy, not ≤ 1 %
   (spikes §2.4). The revised plan:
   - no pool seeds at launch (at most a tiny anti-squat pool, O-1);
   - each collection's share of fee wX (`q_pool`, §5.3) builds its pool toward **≈ 10 SOL a side** in a **narrow range that the
     canister re-centres around the floor**: ±10 % in √price, i.e. `[p/1.21, 1.21·p]` [M, spikes §3.3]. Found while building
     W2: in DAMM v2 the range belongs to the **pool**, so re-centring means a new pool on a dynamic config only Meteora can
     issue. That is open item **S4** (§5.4), with a wide never-re-centred range as the fallback;
   - **busiest collections first**: a collection's pool grows from its own fees, so the 7-day volume ranking is the order.

   At that depth a pool takes a 0.5 SOL buy at ≤ 1 % all-in. It also passes **Jupiter's post-grace liquidity test**:
   - the grace period is **14–30 days from the token's creation** (plan for 14);
   - after it, a token stays routed only if **either** a $500 round trip loses < 30 %, **or** the unit price of a $1,000 buy is
     within 20 % of a $500 buy;
   - Jupiter gates whole tokens: a wX that fails drops every pair through it.

   The old O-1 b (0.5 SOL a side at `[p/4, 4p]`) fails both tests: the $500 buy runs off the range and is refused.
2. **The site's own two-hop buy is required (§5.1, W12).** SOL → wX (DAMM v2) → coin (DBC curve, or the graduated pool), one
   legacy transaction signed by the user's wallet. It always works, with or without Jupiter. Jupiter cannot be the path:
   - it drops a coin still on its curve after the coin's grace period (the normal case for slow, large-`T_wX` collections);
   - it drops a wX whose pool fails the test.

   Measured: 1,033 B and 136 k CU on the curve, 903 B and 88 k CU graduated (spikes §3.5). Builder:
   `solix2::route::two_hop_buy`, checked against the landed bundle.
3. **Pool rent is desk money (§5.2, §12.2).** DAMM v2's `payer` is also the authority over the payer's token accounts, so
   `K_desk[i]` pays the ≈ 0.03 SOL of rent per pool, not `K_main`. It is booked in the desk's capital.
4. **Every new DAMM v2 position needs a fresh position-NFT mint that signs (§1.1, §5.4).** That covers the pool's creation and
   every re-range. It is a throwaway derived key `K_nft[n]`, one more threshold signature (≈ 26 B cycles ≈ $0.035) per opening or
   re-range.
5. **The pool-opening transaction is trimmed to fit a durable nonce (§5.1).** As first designed it was 1,298 B, over the
   1,232-byte limit. The fit:
   - create the desk's wX and wSOL ATAs in an earlier transaction (an ATA can exist before any supply);
   - send mint + wrap + `initialize_customizable_pool`, which is 1,195 B with the nonce;
   - put the memo in the next transaction.

   When the pool opens with a collection's first mint (the tiny anti-squat option of O-1), no wX exists before the pool does,
   so the anti-squat order holds. Builders: `solix2::route::{desk_atas, open_pool, open_pool_memo}`; the size is checked in
   `tests/solix2_builders.rs`.
6. **Signing (§1.1, §6.1).** The changes:
   - sign a message's signers **in parallel**: one signature's latency (≈ 1.2 s on a local replica) instead of three, for the
     same cycles;
   - map signatures by the compiled message's signer order (the fee payer, then the others by key bytes);
   - treat "signature queue full" (20 requests per key locally) as retry-next-tick.
7. **The derivation-path format is fixed (§1.1).** Multi-element paths with fixed-width little-endian integers. Every `K_*`
   address can be computed offline before the canister runs (`ic-ed25519` `derive_mainnet_key`) and pinned in the launch file;
   keys survive upgrades and reinstalls [M, spikes §4].
8. **Rook enforces what DBC does not (§2, §3.1).** DBC accepts a quote mint with a freeze authority, never re-reads the quote's
   mint authority, and accepts a threshold that does not match the curve [M, spikes §2.2]. So the ceremony read-back and the
   canister (`solix2::guard`) require:
   - the quote mint's freeze authority is none;
   - its mint authority is `K_coll[i]`, and the canister never signs a `SetAuthority`, so it is never changed;
   - the config's threshold matches its pinned curve: the curve raises exactly `T_wX` wX by DBC's migration price, and that
     price is the curve's first point.
9. **Leftover coins (§3.3).** Leftover coins per graduated coin depend on the curve: ≈ 53 at `T_wX` = 5, ≈ 9 at 174 (the design
   said ≈ 12.8).
10. **Decision O-1 is rewritten (§14)** to "no seeds; pools grow from fees", with a tiny anti-squat pool as the only launch-day
    option.
11. **Operator go-ahead, 2026-10-10:**
    - **"pools grow from fees"** (item 1 and O-1);
    - **Solana NFT collections join the unlock order after the Ethereum collections** (§16; the collections and their
      `T_wX` are in `deployments/launch/solana-collections.json`, the canister side in `docs/v9/solana-nfts-canister.md`).
12. **W2 is built (§13.1).** The SPL Token, ATA, Memo, Metaplex, DBC and DAMM v2 builders and the account decoders are in
    `autobuyer/canister-rs/src/solix2`. They are checked byte for byte against 42 instructions and 2 bundles that landed in
    LiteSVM on the mainnet program bytes (DBC 0.2.1, DAMM v2 0.2.4, Token Metadata), built by spl-token and by the Anchor
    clients generated from the official IDLs (`solana/tests/solix2_fixtures.rs` →
    `autobuyer/canister-rs/tests/fixtures/solix2/flow.json`).

---

## 0. Summary

**The decision (operator, 2026-10-10, D-l).** Rook deploys **no Solana programs**. That avoids the ≈ 12 SOL program-rent
deposit (`launch-day1-solana.md` §4). The IC canister (`autobuyer/canister-rs`) takes over the programs' job:

- it is the **mint authority** of every collection's wX, an ordinary SPL token;
- it is the **fee claimer** of every Meteora DBC config;
- it keeps the **floor module, wrapper ledger, treasury and raffle books** in its own state;
- it **executes** every money movement with its threshold-Ed25519 keys.

Collection coins trade as **MEMECOIN/wX from day 1** on Meteora DBC and graduate into MEMECOIN/wX DAMM v2 pools with the LP
locked. ROOK stays SOL-paired. A canister-managed **wX/SOL DAMM v2 pool** per collection, built up from fees, lets a wallet buy
SOL → wX → coin in one transaction. The site builds that transaction itself, so it works with or without Jupiter; Jupiter routes
it only within its own limits (§5.1).

**What it costs.** Without pool seeds, the day-1 SOL is ≈ **0.6 SOL** for the 26 wX mints, their metadata and 27 DBC configs,
plus ≈ 0.5 SOL of operating float for the canister (§12). §16 cut this to ≈ 0.2 SOL for five collections. Each wX/SOL pool adds
≈ 0.03 SOL of rent plus its depth. Both are desk money, paid from fees, not at launch (O-1). The depth is not spent: it is
deployed capital that can be recovered (§5.3).

**What it trades away.** On-chain enforcement. With the programs, Solana's validators would have checked every mint, burn and
book on every instruction, and the canister could only act within program limits as custodian and poster. Canister-only, **the
canister's code is the only thing that enforces the books**. Holders trust:

- the code (open source, reproducible build);
- its controller, the 72 h timelock;
- the IC subnet the canister runs on, and the subnet that holds the key;
- two agreeing RPC providers.

What makes this checkable rather than blind trust (§10):

- public queries of every book;
- a memo on every mint and burn transaction;
- a self-check before every signature;
- a reconciler that matches wX supply against custody and SOL on chain.

It is **not** trustless. It is roughly comparable to the programs as they would have launched: their upgrade authority was one
wallet with no delay (D-g). It is weaker than the programs under a Squads timelock.

**Effort.** ≈ **50–68 engineer-days** before an outside audit, at this project's pace (§13). That is ≈ 5–7 calendar weeks for one
person, plus 3 spikes that must pass first (§15).

**Can every collection go live on day 1?** Coins and wX: yes for the 12 collections with a working floor feed and buy pipeline
(11 Ethereum + Bitcoin Frogs). The other 14 Ordinals collections need, at minimum, the runner's multi-collection floor feed.
Without a fresh floor, mint is closed, so no wX exists and their coins cannot trade. Actual purchases for them need the
multi-listing canister work, which is not built (`ordinals-collections.md` §0.3). One-transaction SOL buys exist where a wX/SOL
pool has depth. Pools open and grow from fees, busiest collections first (decision O-1, §14). Until then wX comes from the direct
mint (§6).

---

## 1. Architecture

```
                 Solana (finalized)                                          IC canister (autobuyer/canister-rs)
 ┌───────────────────────────────────────────────┐                ┌──────────────────────────────────────────────┐
 │ wX mints (SPL, 9 dec, mint auth = K_coll[i])  │◀── mint/burn ──│ floor module books per collection (ported     │
 │ module address M[i] (system acct, holds SOL)  │◀── SOL moves ──│   rook_core state/floor.rs)                   │
 │ DBC configs: quote = wX[i], fee_claimer = K_fc│◀── claims ─────│ wrapper ledger: held, eth_backed, tickets     │
 │ DBC pools MEMECOIN/wX ─▶ DAMM v2 MEMECOIN/wX   │                │ coin vault books (per graduated coin)         │
 │ coin vault ATAs (wX) owned by K_coin[c]        │◀── payouts ────│ treasury 90/10, reserve cap, raffle pots      │
 │ wX/SOL DAMM v2 pool[i], position owned by K_dsk│◀── arb/rebal ──│ pool desk (inventory, arb loop)               │
 │ intent addresses (deposits from users)        │── watched ────▶│ intent desk (mint / burn / redeem / NFT out)  │
 └───────────────────────────────────────────────┘                │ solsend lines (durable nonces), solrpc ×2     │
            ▲ site route SOL→wX→coin (Jupiter within its limits)   │ drand (verified BLS), cranks, raffle snapshots│
            │                                                     │ ETH custody (secp256k1), BTC custody (BIP340)  │
 Ethereum / Bitcoin custody addresses ◀── ethflow / frogflow ─────│ Relay legs SOL→ETH / SOL→BTC (relaysol)       │
                                                                  └──────────────────────────────────────────────┘
 off-chain: runner (tick, floor readings) · reconciler (proof of reserves) · monitor · indexer · fueler · site
```

### 1.1 Keys (threshold Ed25519, one master key, derivation paths)

`solkey.rs` already signs on two paths, `b"sol"` and `b"sol-relay"` [R]. Each address below is another derivation path of the
same IC key. A signature costs ≈ 26.15 B cycles ≈ $0.035 whatever the path [M, `solana-migration.md` §2.4].

| Key | Path | Role | Why it is separate |
|---|---|---|---|
| `K_main` | `[b"sol"]` (exists) | fee payer and nonce authority of every line; custodian of Solana NFTs (later) | unchanged; holds only operating SOL |
| `K_relay` | `[b"sol-relay"]` (exists) | Relay deposits (audit A5-03) | unchanged |
| `K_coll[i]` | `[b"sol", b"coll", i: u32 LE]` | **module address `M[i]`** (holds collection i's backing, reserve, surplus and revenue SOL) and **mint authority of wX[i]** | one collection's SOL is one on-chain balance anyone can read against its books; a bug in one collection's path cannot spend another's |
| `K_fc` | `[b"sol", b"claimer"]` | `fee_claimer` and `leftover_receiver` of every collection DBC config; owner of the partner LP position NFTs | holds fee wX between claim and booking; one key, so a config can name it before any collection exists |
| `K_coin[c]` | `[b"sol", b"coin", coin mint: 32 B]` | owner of coin c's backing vault ATA (wX) | each coin's backing is one on-chain balance |
| `K_desk[i]` | `[b"sol", b"desk", i: u32 LE]` | collection i's wX/SOL pool position and its arb inventory; **pays its pool's rent** (DAMM v2's `payer` is the token authority, spikes §2.3) | keeps desk (treasury) money apart from backing |
| `K_tre` | `[b"sol", b"treasury"]` | treasury SOL (reserves pot, ops, raffle pots as books) | as above |
| `K_int[n]` | `[b"sol", b"intent", n: u64 LE]` | one deposit address per user intent (§6) | attribution without memos |
| `K_nft[n]` | `[b"sol", b"nft", n: u64 LE]` | **throwaway position-NFT mint** of a new DAMM v2 position (a pool's opening, every re-range): signs once, as a new account, and holds nothing (spikes §2.4 item 3) | DAMM v2 makes the NFT mint sign; a fresh `n` per position, never reused |

**Path encoding (pinned, spikes §4).** Paths are **multi-element**: each element is one HKDF step, and integers are
**fixed-width little-endian** (`u32` for collection indexes, `u64` for counters). The encoding is permanent: changing it changes
every address. With this form, `K_main`'s key and chain code derive every child's public key offline (harmless). The two
existing single-element paths (`sol`, `sol-relay`) keep their addresses.

All of these are on the ed25519 curve, so they are ordinary wallets. Two consequences:

- The raffle's "exclude off-curve owners" rule (`raffle.rs`) no longer excludes Rook's own accounts. The snapshot must exclude
  every canister-derived address explicitly (§9.3).
- Every transaction that moves money held by `K_coll`, `K_coin`, `K_desk` or `K_int` carries two or three signatures:
  `K_main` pays the fee and advances the nonce; the owner key moves the funds. Fees never come out of a backing address. A pool
  opening or re-range carries four: `K_main`, `K_coll[i]` (the mint), `K_desk[i]` and `K_nft[n]`.
- **Signing is parallel** (spikes §4): the canister signs a message's signers concurrently, which takes one signature's latency
  (≈ 1.2 s on a local replica) for the same cycles. It maps each signature by the compiled message's signer order: the fee payer
  first, then the others by key bytes, not by role. A "signature queue full" rejection (20 queued requests per key locally;
  the mainnet value is unknown and shared across the signing subnet) is refunded, and the operation retries next tick. At three
  signatures a transaction, at most ≈ 6 transactions sign at once, which bounds the intent desk's and the arb loop's throughput
  per tick.

`sign_message` today refuses a message the key is not the **only** signer of [R, `solkey.rs` header]. That rule becomes "every
signer of the message is a canister key, and the canister signs each one in turn". This is a deliberate change to an
audit-sensitive rule, and goes on the audit list.

Deriving public keys: one `schnorr_public_key` call per new path, cached in state. Offline derivation works **[M, spikes §4]**:
`ic-ed25519` 0.6.0 `derive_mainnet_key(Key1, canister_id, path)` gives every `K_*` address byte-equal to the canister's answer,
so the launch file can pin them all before the canister runs. Keys are stable across upgrades and reinstalls; they depend only
on the master key, the canister id and the path. Compare once on the first mainnet run.

**Built (W3, 2026-10-10).** The table is `solkey::KeyPath` (`derivation()` is the only place the encoding lives; text form
`main`, `relay`, `coll:<i>`, `claimer`, `coin:<mint>`, `desk:<i>`, `treasury`, `intent:<n>`, `nft:<n>`).
`cargo run --example sol_keys -- --canister <id> [--key key_1|test_key_1]` derives every address offline for the ceremony;
the operator method `sol_address_of(path)` gives the canister's own answer to compare. `tests/solkey_paths.rs` pins the bytes and
re-derives S3's measured addresses. The signing rule is `solkey::signer_paths`: every signer of the compiled message is a key the
caller named (or `K_main`), `K_main` is the fee payer (or the Relay key signs alone), the signed size is ≤ 1,232 bytes; all checked
before the simulation and before any signature; then every signature is asked at once (`join_all`), and a full queue
(`solkey::QUEUE_FULL`) saves nothing. The four send lines (§13.1 W3: money, crank, mint/burn desk, pool desk) each have their own
durable nonce, seed `<tag>:<nonce_seed>` (the money line keeps `nonce_seed`); this seed is permanent too.

### 1.2 What lives where

| Thing | On Solana | In the canister |
|---|---|---|
| wX balances, supply | SPL mint and token accounts | expected supply, from the ledger (§10.1) |
| Collection SOL | `M[i]` balance | `backing`, `reserve`, `surplus`, `revenue`, advance in transit (ported `Module`) |
| Items held, tickets | — (the NFTs are on Ethereum or Bitcoin) | wrapper ledger, items, tickets, deliveries |
| Coin backing | wX ATA of `K_coin[c]` | `CoinVault` book (paid out, burned) |
| Treasury, raffle | SOL at `K_tre`; prize wX at an ATA of `K_tre` | shares, pots, rounds, snapshots |
| Coins | DBC and DAMM v2 (Meteora) | read-only (pools, fees claimable) |

---

## 2. wX: classic SPL tokens issued by the canister (item 1)

- **Program:** classic SPL Token (`Tokenkeg…`), **9 decimals**, so 1 wX = 10⁹ base units = 1 NFT. This matches rook_core's
  `ONE` [R, `rook-core-as-built.md` §3].
- **Mint authority:** `K_coll[i]`. **Freeze authority: none.** That is what rook_core's `["wx_mint", wrapper]` had [R].
  Jupiter flags tokens whose mint authority is not renounced. wX can never renounce it, because it is issued on demand.
- **Metadata:** Metaplex Token Metadata, `create_metadata_accounts_v3`, with the name, symbol and uri from
  `deployments/launch/nft-pilot.json` (wMILADY … wNATCAT, 26 symbols). It has no creators, 0 royalty and `is_mutable = true`,
  and the **update authority is `K_coll[i]`**. This is the same choice rook_core §9.6 made for its wrapper PDA: renaming a wX
  needs the canister, and so the timelock. Cost: 0.0051156 SOL rent + 0.01 SOL Metaplex fee + 0.0014616 SOL for the mint =
  **0.0165772 SOL per collection** [M, `rook-core-as-built.md` §2, §9.6].
- **Who creates them:** the ceremony, not the canister. The canister does not pay cycles for setup, and the result is checked
  on chain. In one transaction, the ceremony hot key (`rook-ceremony`):
  1. creates the mint with itself as mint authority;
  2. creates the metadata with update authority = `K_coll[i]` (Metaplex lets the creator name another update authority);
  3. calls `SetAuthority(MintTokens → K_coll[i])`.

  After that the hot key has no power. `rook-ceremony` reads back:
  - mint authority = `K_coll[i]`;
  - freeze authority none;
  - supply 0;
  - decimals 9;
  - metadata update authority = `K_coll[i]`.

  The mint addresses are pinned in the launch file. A ceremony that cannot do this leaves the work to the canister: ≈ 2
  signatures per collection, ≈ 1.4 T cycles in all.

  Measured: one transaction of 753 B, 3 signatures, 42–46 k CU [M, spikes §2.1].
- **What DBC does not check, Rook does** [M, spikes §2.2]. DBC accepts a quote mint with a freeze authority and never re-reads the
  quote's mint authority. So the canister checks, at install and before it trusts a collection, the same things the ceremony
  reads back (`solix2::guard::check_wx_mint`): initialized, 9 decimals, mint authority `K_coll[i]`, **freeze authority none**.
  It also **never signs a `SetAuthority`**, nor an `Approve`, nor a Metaplex update that moves the update authority
  (`guard::refuse_forbidden`, run on every message before any signature). The mint authority is therefore never changed or
  renounced while the canister lives.
- **Builders: built (W2, 2026-10-10).** `autobuyer/canister-rs/src/solix2`:
  - `spl`: `CreateAccount`, `InitializeMint2`, `MintTo(Checked)`, `Burn(Checked)`, `Transfer(Checked)`, `CloseAccount`,
    `SyncNative`, `SetAuthority` (for the ceremony; refused by the guard), ATA `Create` / `CreateIdempotent`, Memo v2;
  - `mpl`: `CreateMetadataAccountV3`, and `UpdateMetadataAccountV2` as a rename only.

  Each is checked byte for byte against an instruction that landed in LiteSVM on the mainnet program bytes
  (`tests/solix2_builders.rs`).

---

## 3. Collection coins on DBC with quote = wX (item 2)

### 3.1 What changes in the config

The day-1 tooling stays (`launch-params.mjs` `meteora` section, `day1-pins.json`, `rook-ceremony day1`). Per collection config:

| Field | Before (D-e) | Canister-only |
|---|---|---|
| `quote_mint` | wSOL | **wX[i]** |
| `fee_claimer`, `leftover_receiver` | rook_backing PDA `["claimer", listing]` | **`K_fc`** (a real key the canister signs with) |
| `migration_quote_threshold` | 85 / 170 / 255 / 510 SOL (tier) | **`T_wX[i]` whole wX** (§3.2) |
| `sqrt_start_price`, curve | one per tier | **one per collection**: `design_curve.mjs 1e9 20 <T_wX> 50 6` with the quote decimals = 9 (same as wSOL) |
| `collect_fee_mode` (curve) | quote | quote: **fees in wX** |
| migrated pool | DAMM v2, LP permanently locked 50/50, fee in quote, no compounding | unchanged: **MEMECOIN/wX** DAMM v2, LP locked, fees in wX |
| migration fee | 50 % to the partner | unchanged: **50 % of the raise, in wX**, to `K_fc` |
| anti-sniper, supply, metadata | Q1 schedule, 1B fixed, immutable | unchanged |
| `enable_first_swap_with_min_fee` | false | **true** (operator decision 2026-10-11, pump.fun's "dev buy"): the creator's first buy, made in the launch transaction, pays the schedule's end fee (1.000004 %), not the anti-sniper fee; everyone else keeps the schedule. `dbc::pins::FIRST_SWAP_WITH_MIN_FEE`, below |

`rook_backing::register_partner_config`'s checks become an off-chain check in `rook-ceremony` and a canister check at install.
The canister decodes each pinned config at start (`solix2::decode::DbcConfig`) and refuses to treat a pool as Rook's if anything
differs (`solix2::guard::check_config`):

- version 0;
- quote = wX[i];
- `fee_claimer` = `leftover_receiver` = `K_fc`;
- timestamp activation;
- LP locked;
- no creation fee;
- creator migration share 0;
- Rook's fee schedule, fixed 1B supply and migrated-pool fee;
- `enable_first_swap_with_min_fee` = 1 (byte 365): a config without it is not the one the canister creates. What it does, from
  DBC 0.2.1's source (`process_swap.rs`, the deployed `f552f20`; the mainnet program bytes = `solana/tests/fixtures/dbc.so`, re-dumped
  2026-10-11): a swap pays the scheduler's minimum fee (`get_min_base_fee_numerator`, the fee after the last period: 500,000,000 −
  60 × 8,166,666 = 10,000,040 / 1e9) instead of the time-based one only if (a) the config has the flag, (b) it is the pool's first
  swap ever (`VirtualPool::has_swap == 0`; set by every swap), (c) the swap names the Instructions sysvar as its one extra
  remaining account, (d) it is a top-level DBC instruction (not a CPI), with no referral account, and (e) an earlier top-level
  instruction of the same transaction is DBC's `initialize_virtual_pool_with_spl_token` / `…_token2022` /
  `…_token2022_transfer_hook` naming this pool (its account 5). No signer is checked: whoever builds the pool's creating
  transaction gets it, and nobody else can, since the pool does not exist before. The amount is not capped (a creator could take
  the whole curve to the threshold at 1 %). If (c)–(e) fail the swap is not refused: it pays the normal fee. DBC accepts the
  sysvar on any swap (at most one non-transfer-hook remaining account, which must be the sysvar), so it is harmless elsewhere.
  Proven on the mainnet bytes: `solana/tests/first_swap_min_fee.rs` (init + swap + sysvar: 1.000004 %; a later swap at t0, with or
  without the sysvar: 50 %; init + swap without the sysvar: 50 %; a first swap in a later transaction: 50 %; the flag off: 50 %).
  The site's launch (`web-rook/src/rook/sol/trade.ts` `launchIxs`) passes the sysvar on its first buy;
- **the threshold matches the curve.** DBC accepts a threshold that does not fit the curve (1 wX on the 3-wX curve was
  accepted, spikes §2.2). So the canister requires all three of:
  - the threshold is `T_wX[i]` whole wX, and the curve is the one pinned for that `T_wX`;
  - buying along the stored curve from `sqrt_start_price` to DBC's own `migration_sqrt_price` raises the threshold (within
    0.1 %);
  - that migration price is the curve's first point (`design_curve.mjs` puts graduation there).

  All 20 curves of §3.2 pass; a mismatched threshold fails.

The quote mint's own rules (freeze authority none, mint authority `K_coll[i]`) are checked on the mint, §2.

**ROOK** keeps its SOL-paired genesis config: creator share 100 % to the fund wallet, migration fee 0, no backing. It is
unchanged except that its claimer is no longer a program PDA. It was already the fund wallet [R, D-g].

**A config cannot be re-pointed.** `fee_claimer` is written once and there is no close instruction [M, `launch-day1-solana.md`
§2]. If `K_fc`'s address were ever lost (§13.2: the canister deleted), every config's fees and every future migration fee would
be lost with it. That is the same exposure the reserved program keypairs carried, moved to the canister's existence.

### 3.2 Graduation thresholds in wX

`graduation-tiers.md`'s rule was that a graduation buys ≥ 1.33 NFTs from SOL. With wX quote, the migration fee **is** wX. Half
the raise lands in the coin's vault as already-backed units, and NFT purchases are driven by wX minting (the module's buy
signal), not by the graduation. The rule becomes:

> `T_wX = max(3, ⌈tier_SOL / floor_SOL⌉)` whole wX at day-1 floors. The coin's vault then gets ≥ 1.5 wX, and the raise is
> worth about the old tier in SOL on day 1.

Measured floors (`deployments/launch/graduation-tiers.json`, 2026-10-10) [M], `T_wX` [A]:

| collection | floor (SOL) | T_wX | ≈ SOL | | collection | floor (SOL) | T_wX | ≈ SOL |
|---|---|---|---|---|---|---|---|---|
| Milady | 18.61 | 5 | 93 | | NodeMonkes | 17.44 | 5 | 87 |
| Azuki | 13.71 | 7 | 96 | | OMB | 13.16 | 7 | 92 |
| MAYC | 18.56 | 5 | 93 | | Bitcoin Puppets | 7.70 | 12 | 92 |
| Pudgy Penguins | 59.00 | 5 | 295 | | Bitcoin Punks | 2.36 | 37 | 87 |
| BAYC | 136.84 | 4 | 547 | | Natcats | 2.14 | 40 | 86 |
| Doodles | 5.95 | 15 | 89 | | Runestone | 1.04 | 82 | 85 |
| Moonbirds | 8.90 | 10 | 89 | | Bitcoin Weirdos | 0.49 | **174** | 85 |
| CloneX | 3.38 | 26 | 88 | | CENTS | 3.87 | 22 | 85 |
| Lil Pudgys | 5.83 | 15 | 88 | | RSIC | 0.56 | **152** | 85 |
| Checks | 3.47 | 25 | 87 | | Pizza Ninjas | 6.56 | 13 | 85 |
| Identity MD | 40.39 | 5 | 202 | | Quantum Cats | 7.86 | 11 | 87 |
| Bitcoin Frogs | 3.01 | 29 | 87 | | Wizards of Ord | 4.06 | 21 | 85 |
| | | | | | Taproot Wizards | 35.35 | 5 | 177 |
| | | | | | Bitcoin Wizards | 10.15 | 9 | 91 |

**After day 1 the market cap floats with wX.** A coin's graduation market cap in SOL moves with the NFT floor. That is the
point of wX pairing, but it is also new risk for coin traders.

**The cheap-collection problem.** Every wX inside a DBC curve, a pool or a coin vault is a SOL-backed unit (`eth_backed`) until
the canister has bought NFTs for it. One Weirdos curve needs 174 wX. rook_core's unit caps were sized for a 1-ETH-class
collection (`max_eth_backed` 40, public cap 10 or the posted depth, 5 units an hour) [R, `state/floor.rs`]. They would stop a
single Weirdos curve from filling. §4.3 makes the caps value-based. Even so, Rook is **structurally short NFTs** by the whole
float until purchases catch up, at ≤ `max_items_per_day` (3 at stage 0).

Burns never pay more than backing, so the module stays solvent in SOL. What suffers is "1 wX → a real NFT": a redemption needs
a held item. The purchase pipeline converts float into items at the rate limits. Decision **O-4**: how to treat collections
with floors under ≈ 2 SOL (Runestone, Weirdos, RSIC; also Punks and Natcats).

### 3.3 What the canister does around a coin

These replace solcrank's rook_backing cranks [R, `solcrank.rs`]. The pass, budget and interval framework is reused.

| Event | Canister action (one transaction unless noted) | Signers |
|---|---|---|
| a new DBC pool under a Rook config | record a `CoinVault` book (pool, coin mint, `launched_at` = `activation_point`, creator) — no rent, no transaction | — |
| the pool migrated (`is_migrated`) | create the ATA of `K_coin[c]` for wX[i] (idempotent); DBC `withdraw_migration_fee(flag 0)` with `token_quote_account` = that ATA (the wX goes straight to the coin's vault, as the parked D-j `back` did [R, branch `solana-wx-paired-wip`]); Memo `rook:<i>:back:<c>` | `K_main`, `K_fc` |
| partner curve fees ≥ threshold | DBC `claim_trading_fee` → wX to `K_fc`'s ATA; book as treasury income in wX[i] | `K_main`, `K_fc` |
| partner LP fees ≥ threshold | DAMM v2 `claim_position_fee` on the partner position NFT that DBC minted to `K_fc` | `K_main`, `K_fc` |
| leftover coins (curve-dependent: ≈ 53 at `T_wX` = 5, ≈ 9 at 174 [M, spikes §2.1]) | `withdraw_leftover` (permissionless) pays `K_fc`'s coin ATA; then **burn them** (raises every coin's backing per unit; the open rook_backing question, A3-04, closed this way) | `K_main`, `K_fc` |

The canister builds these by hand, as the site's `dbc.ts` does: there is no CPI-depth limit and no Anchor. The builders exist
(W2, `solix2::dbc` and `solix2::damm`), with the account lists in the order of the official IDLs (DBC 0.2.1, DAMM v2 0.2.4):

- DBC: `create_config` with Rook's pinned parameters (`dbc::rook_wx_config`), `initialize_virtual_pool_with_spl_token`,
  `swap`, `swap2`, `claim_trading_fee`, `withdraw_migration_fee`, `partner_withdraw_surplus`, `withdraw_leftover`,
  `migration_damm_v2`;
- DAMM v2: `initialize_customizable_pool`, `create_position`, `add_liquidity`, `remove_liquidity`, `remove_all_liquidity`,
  `claim_position_fee`, `close_position`, `swap`, `swap2`;
- decoders for DBC `PoolConfig` and `VirtualPool`, DAMM v2 `Pool` and `Position`, and SPL `Mint` and token `Account`.

Every one was checked byte for byte against the spike flow re-run with fixed keys in LiteSVM [M].

---

## 4. The floor module inside the canister (item 3)

### 4.1 Port, not rewrite

rook_core already separates the pure book-keeping (`impl Module` in `state/floor.rs`, 731 lines) from the Anchor handlers
(`instructions/floor.rs`). The `impl Module` covers:

- `post_floor`, `floor_bounds`, `price` (fills TWAP), `spreads`;
- `mint`, `burn`, `release`, `tier_prices`;
- `advance`, `settle_advance`, `return_advance`, `item_arrived`, `take_one`;
- `skim`, `fund_reserve`, `snapshot_cap`, `max_item_cost`, `net_units`, `escaped`, `pause_floor`.

Those functions call only `mul_div`, `mul_div_ceil`, `add`, `sub`, the clock constants and `RookError`. The port:

| rook_core source | canister module (new) | change |
|---|---|---|
| `state/floor.rs` `Module` + `params.rs` + `math.rs` + `clock.rs` + `units.rs` | `floor.rs` | `anchor_lang::Result` → `Result<_, FloorError>`; `#[account]` → `CandidType + Serialize`; the logic is unchanged. **Drop `convert`** (no SOL-quoted graduation) and keep `split`, `intake` and `reserve_units` |
| `state/wrap.rs` + `instructions/wrap.rs` (ledger, items, held markers, tickets, resolve, refund, deliveries, mint limit) | `wrapledger.rs` | the account checks become map lookups; the ticket cursor and `refund` rules are kept (A2-04: no refund of a pinned ticket whose round is proven) |
| `state/treasury.rs` + `instructions/treasury.rs` (shares, 24 h changes, `allocate`, `distribute`, reserve cap, raffle bps) | `treasury.rs` | pagination goes (one canister call distributes to every collection); the 24 h delays stay |
| `state/raffle.rs` + `instructions/raffle.rs` (phases, `fund_prize`, commit/draw/claim/expire, leaf, ticket) | `raffle.rs` (exists: snapshot, tree, `raffle_snapshot` query) | phases and rules move in; the draw uses the canister's **verified** drand round |
| `hashing.rs` | already ported (`frogbuy::leaf_of`, `verify_proof`, taproot checks) | — |
| registry roles (owner, guardian, keeper, converters, poster switches, roots +7 d) | `governance.rs` | owner → the controller (timelock upgrade) plus a short list of **guardian principals** with stop-only methods (§10.4); 7-day root changes stay as canister-enforced delays |
| wake desk (`request`, `reclaim`, `forfeit`) | `wake.rs`, or dropped | decision O-11 |

**Tests come with it.** All of these are ported to plain `cargo test` against the canister crate:

- `tests/suite/floor.rs` (22), `props.rs` (5 random sequences, with the invariants checked after every step);
- `wrap.rs` (13), `treasury_wake.rs` (17), `raffle.rs` (10);
- `audit_a1_core.rs`, `audit_a2_core.rs`.

They were LiteSVM tests that build accounts. The scenario logic ports; the account plumbing is deleted. The 400 vectors in
`rook-core-vectors.json` are already checked by the canister (`tests/solana.rs::the_ported_floor_rules_and_hashing_match_rook_cores_own_vectors`)
[R]. They extend to `mint`, `burn` and `release` from `vectors.rs`.

**Why port rather than rewrite.** The logic was audited twice: as `FloorModule.sol` and in `docs/v9/audit-2026-10` (A1, A2).
Keeping it line for line keeps that review useful. The new audit then only has to cover the **seams**: where books meet SOL and
wX on chain (§4.4).

### 4.2 Floor input

Unchanged in substance. Ethereum floors come from the canister's own reads. Ordinals floors come from the runner's
`frog_floor_reading` (`ops/src/solfloor.ts`) [R]. Both now go into the internal `post_floor` instead of a rook_core
transaction, so posting costs no signature and no SOL.

The clamps (±`max_move_bps`/h, ±20 %/day), the 3 h TTL, the 7-day stale escape and the poster pause all stay. Trust is the same
as before: the canister was already the poster.

### 4.3 Parameters: from units to value

Every unit-denominated parameter is re-derived per collection from SOL at listing, written by `launch-params.mjs` into the
canister's `Init`:

| parameter | rook_core default | canister-only proposal [A] |
|---|---|---|
| `mint_per_hour`, `burn_per_hour` | 5 wX | `max(5 wX, ⌈250 SOL / floor⌉)` |
| `max_public_eth_backed` | 10 wX | `max(10 wX, ⌈1.5 × T_wX × open curves allowed⌉)`; the `depth_items` term stays as a brake on the **buy pipeline**, but no longer on minting (decision O-4) |
| `max_eth_backed` | 40 wX | `max(40 wX, ⌈2,000 SOL / floor⌉)` [A, sized to ≈ 20 graduations' float] |
| `max_items_per_day` | 3 (stage 0) | 3, then the stage plan; up to 32 |
| `reserve_bps` | 2,500 | keep (decision O-5: with wX quote every unit enters by a public mint, so 25 % of all SOL-backed wX is never advanced) |
| spreads, `conv_bps`, `fill_band_bps`, TTLs | as built | unchanged |

### 4.4 The seams: books against chain

The canister holds the books. The chain holds the money. Four rules connect them, each enforced in code and each tested:

1. **Book first, then sign, then confirm.** An operation (a mint, burn, advance or payout) is a `Pending` entry, with its
   effect reserved in the books (`reserved_out` lamports, `pending_mint` units), **before** its bytes are signed and saved
   (`solsend::prepare`). It becomes final only when `getSignatureStatuses` shows it finalized. If the transaction can never land
   (nonce moved and signature unknown, read in that order: Q3 §4.3), the reservation is undone.

   This is `state.rs`'s rule ("nothing that spends is attempted until the intent to spend is durably recorded") applied to
   Solana [R].
2. **The supply guard.** Before signing any `MintTo` of wX[i], the canister reads the mint's supply at finalized and requires:

   `supply + Σ pending mints + amount ≤ held·10⁹ + eth_backed + pending_unit_allowance`

   that is, the ledger's own count. Otherwise it stops that collection and raises an incident. A bug that would over-mint stops
   at the signature.
3. **The balance guard.** Before signing anything that moves SOL out of `M[i]`, it requires:

   `balance(M[i]) − rent − amount ≥ books(i) − released_by_this_op`

   with the balance read at finalized.
4. **The memo.** Every canister transaction that changes a supply or a backing balance carries `rook:v1:<coll>:<kind>:<entry id>`.
   Anyone can rebuild the full ledger from Solana history and the canister's `ledger_log` query (§10.1).

**Unsolicited transfers.** Anyone can send SOL to `M[i]` or wX to a vault ATA. Such money is never counted as books. It shows
up as "unbooked" in the proof of reserves, and is swept into `surplus` (SOL) or a coin's vault (wX) at the next daily
reconcile, with an event.

---

## 5. The wX/SOL pool per collection (item 4)

### 5.1 Purpose and type

The pool gives wX a market price. A wallet then buys a coin with SOL in **one transaction**: SOL → wX (this pool) → coin (DBC
curve, or the migrated DAMM v2 MEMECOIN/wX pool).

**The site builds that transaction itself, and must** [M, spikes §3.4–3.5]:

- one legacy transaction: 1,033 B and 136 k CU on the curve, 903 B and 88 k CU graduated, no lookup table;
- builder: `solix2::route::two_hop_buy`.

It always works, with or without Jupiter. Jupiter routes the same path today (DBC curves quoted in any SPL token, thin DAMM v2
pools, and SOL → X → coin as two hops [M, spikes §3.1]), but **only within each token's grace period** of ≈ 14–30 days from
the token's creation (plan for 14). After that:

- a wX stays routed only if its pool passes **one** of Jupiter's tests:
  - a $500 round trip loses < 30 %;
  - the per-unit price of a $1,000 buy is within 20 % of a $500 buy's;
- Jupiter gates whole tokens, so a wX that fails takes every pair through it off Jupiter;
- **a coin still on its DBC curve is removed** until it graduates. For the slow, large-`T_wX` collections that is the normal
  case.

The **depth comes from the canister's mint and burn**, not from the pool. The canister arbitrages the pool back to its band
after every trade (§5.4). So the pool only has to absorb **one transaction's worth** of buying, and, to stay on Jupiter, a
$1,000 (≈ 9 SOL) buy without leaving its range.

**Type: DAMM v2**, one pool per collection, created by `K_desk[i]` through `initialize_customizable_pool`:

- base fee 0.5 % fixed;
- `collect_fee_mode` = fees in SOL;
- timestamp activation, active at once;
- a **narrow range re-centred by the canister around the floor**: ±10 % in √price, i.e. `[p₀/1.21, 1.21·p₀]`, where `p₀` is
  the floor at opening (`solix2::route::range_around(s, 11, 10)`).

**Why narrow** [M, spikes §2.4, §3.3]. On `[p/4, 4p]`, a buy of Δ SOL moves the pool's price by ≈ Δ / (SOL a side), so
`[p/4, 4p]` needs ≈ 50 SOL a side for ≤ 1 % on 0.5 SOL. `[p/1.21, 1.21p]` at ≈ 10 SOL a side gives ≤ 1 % on 0.5 SOL buys and
passes both Jupiter tests (round trip 1.0 %; $1k vs $500 +3.9 %). The tighter `[p/1.1, 1.1p]` (±10 % in price) gives 0.97 % on
0.5 SOL at only 5 SOL a side. It would not hold a $1,000 (≈ 9 SOL) buy at that depth [A: not measured; `[p/1.21, 1.21p]` at
5 SOL already refused it]. The cost of a narrow range is that re-ranges are routine (§5.4).

The position is **not permanently locked** (decision O-2), so when the floor leaves the range the desk can withdraw it and
re-open around the new floor. In DAMM v2 the range is the pool's, so that means a **new pool** on a dynamic config Meteora
issues for the desk. A new position on the old pool would keep the old range (§5.4, open item S4). Either way, the new
position's NFT mint `K_nft[n]` signs.

**Rent is desk money.** DAMM v2's `payer` is also the authority over the payer's token accounts, so `K_desk[i]` pays the pool's
≈ 0.03 SOL of rent [M, spikes §2.3]. It is booked as desk capital (§5.2), never as `K_main`'s operating SOL.

**The opening transaction, trimmed** [M, spikes §2.3]:

- as first designed (desk ATAs + mint the seed + wrap + create the pool + memo, behind a durable-nonce advance) it was 1,298 B,
  over the 1,232-byte limit;
- the fit: create the desk's wX and wSOL ATAs in an earlier transaction (an ATA can exist before any supply), then send mint +
  wrap + `initialize_customizable_pool` (**1,195 B** with the nonce), and put the memo in the next transaction;
- signers: `K_main`, `K_coll[i]`, `K_desk[i]`, `K_nft[n]`;
- builders: `solix2::route::{desk_atas, open_pool, open_pool_memo}`.

**Squatting.** The DAMM v2 customizable-pool address is one per pair, and anyone holding wX can create it first, at any price
**[M, `wx-pools-design.md` §5]**. Canister-only can remove the problem: wX supply is 0 until the canister's first mint. If the first wX ever
minted for a collection is the desk's pool seed, in the **same transaction** as the pool's creation (trimmed as above), nobody
else can hold wX before the pool exists. After that, other pools on static configs are harmless; Jupiter routes by liquidity.

With **no pool at launch** (O-1, revised), wX first comes from the direct mint (§6), and a holder could create the
customizable pool first, at any price. That is harmless to the books: the arb loop never touches a pool it does not own, and
arbitrageurs price a squatted pool against the canister's mint and burn. But it takes the one `cpool` address, and the desk's
pool then goes on a static DAMM v2 config instead. O-1 therefore keeps a **tiny anti-squat pool** (opened with the first mint)
as the one launch-day option.

### 5.2 Not double-counting backing

`wx-pools-design.md` §2 sets the rule: **every wX in the pool is backed in the module like any other unit, and the pool's SOL
side is SOL that no unit claims.** Canister-only, this holds by construction:

- The desk gets its wX **only through the module's public mint**. It pays the mint price into `M[i]`, the units go into
  `eth_backed`, and `intake` books them. To the books, the desk is an ordinary holder.
- The desk's SOL is **treasury money**. It is held at `K_desk[i]`, booked in the treasury as "pool desk capital", and never in
  `backing`, `reserve`, `surplus` or `revenue`.
- So seeding a pool with `S` SOL a side costs `2S` of desk money: `S` buys `S/p` wX from the module (which then holds `S` more
  SOL against `S/p` more units), and `S` goes in as SOL.
- The pool's rent (≈ 0.03 SOL: pool, vaults, position, NFT) is also desk money: `K_desk[i]` pays it (spikes §2.3). It comes back
  only partly: `close_position` returns the position's rent, and the pool and its vaults stay open.

Nothing is counted twice. The desk's equity is its LP position (plus inventory), and it can be recovered by withdrawing and
burning (§5.3).

### 5.3 Where the desk money comes from

| Source | When | Note |
|---|---|---|
| ~~Day-1 seed from the operator or fund wallet~~ | **none** (revised O-1, operator go-ahead 2026-10-10: pools grow from fees) | at most a tiny anti-squat pool from the bootstrap float (O-1) |
| **Partner fees in wX** (DBC curve fees + migrated LP fees, §3.3): **the main source** | from the first trades | `q_pool` (proposed 25 %) of collection i's fee wX goes to `K_desk[i]` while its pool is under target depth: `q_pool/2` of value as wX (already backed: fees are existing units), and the other half burned at the module for SOL. The rest is cashed to the treasury (§9.1). Since each collection's pool grows from its own fees, **the busiest collections get depth first** |
| **Pool trading fees** (0.5 %, in SOL, 80 % to the LP after Meteora's 20 % **[M]**) | ongoing | stay in the desk until target depth, then to the treasury |
| **Arb profit** (§5.4) | ongoing | stays in the desk (inventory) |

**Target depth (revised after S1/S2).** The earlier sizing here (≈ 2.5 SOL a side at `[p/4, 4p]` for ≤ 1 % on 0.5 SOL) was
≈ 20× too thin [M, spikes §2.4]. The target is now **≈ 10 SOL a side at `[p/1.21, 1.21·p]`** (≈ 20 SOL of desk money per
collection, plus ≈ 0.03 SOL rent). That gives:

- ≤ 1 % all-in on a 0.5 SOL buy;
- room for a $1,000 buy inside the range;
- both of Jupiter's post-grace tests passed [M, spikes §3.3].

**How a pool grows from fees** (O-1, revised):

1. A pool opens when collection i's desk holds ≈ 0.5 SOL a side plus rent. At that depth in the narrow range a 0.01 SOL buy
   pays ≈ 0.7 % all-in and a 0.1 SOL buy ≈ 2.3 % (spikes §2.4). Larger buys go through the direct mint, or pay slippage that the
   arb recovers for the desk.
2. `add_liquidity` from each fee batch until ≈ 10 SOL a side; then `q_pool` falls to 0.
3. Re-centre around the floor as the arb loop needs it (§5.4).

Until a pool passes Jupiter's test, the site's own route (§5.1) and the direct mint serve buyers. Jupiter needs a depth that a
new collection's fees cannot reach within the 14-day grace, so a collection should expect to **leave Jupiter after its grace
and come back** once its pool reaches the target (Jupiter re-checks every 30 minutes) [W].

| Stage | Desk money for collection i | What buyers get |
|---|---|---|
| no pool (launch default) | 0 | the direct mint (§6, ≈ 40–150 s); the site's two-hop buy where a coin has graduated into COIN/wX and the buyer holds wX |
| tiny anti-squat pool (O-1 option) | ≈ 0.03 SOL rent + ≤ 0.1 SOL a side | price discovery and the address kept; one-transaction buys only to ≈ 0.01 SOL at ≈ 2–3 % [A] |
| opened from fees | ≈ 1 SOL + rent | one-transaction buys: ≈ 0.7 % at 0.01 SOL, ≈ 2.3 % at 0.1 SOL |
| target | ≈ 20 SOL + rent | ≤ 1 % to 0.5 SOL; passes Jupiter's tests |

### 5.4 Keeping the pool near the floor: the arbitrage loop

With mint price `m = mint_base × (1 + s_mint)`, burn price `b = min(price, burn_target) × (1 − s_burn)` and pool fee `f`, the
**band** is `[b/(1+f), m/(1−f)]` (`wx-pools-design.md` §7). Each check:

- **Price above the band:** the desk mints `x` wX at the module (public mint rules, caps and buckets) and sells them into the
  pool, with `x` chosen so the pool lands at `m/(1−f)`. This is closed form for a concentrated constant-product position. The
  buyer's slippage becomes the desk's profit.
- **Price below the band:** the desk buys `x` wX from the pool with SOL and burns them at the module (public burn rules).
- **Inside the band:** nothing.

Bounds [A]:

- **Profit floor:** expected profit ≥ 0.0005 SOL plus the transaction's SOL fee plus its cycles (≈ 2 signatures ≈ $0.07).
- **Size:** per arb ≤ the desk's free inventory and ≤ 25 % of the pool's scarcer side; ≤ 12 arbs an hour per collection;
  inventory ≤ 10 % of the collection's backing.
- **Slippage:** `minimum_amount_out` = the quoted amount less 0.3 %. If a sandwich or race moves the pool, the swap fails and
  costs only the fee.
- **Stale floor or poster paused:** mint is closed, so the up-arb stops. The down-arb still runs while burns are open (fresh,
  `kd` switch, or escaped). A pool can then drift above the band, and the site says so.
- **Atomicity:** mint and swap are **one transaction**, signed by `K_main`, `K_desk[i]` and `K_coll[i]`, so the desk never holds
  unsold inventory because one leg landed without the other. The burn side is two steps: the swap lands, then the burn. The
  wX waits in the desk between them.

**Cadence.** `getMultipleAccounts` with `dataSlice` (only `sqrt_price`, 16 bytes per pool) for every open pool:

- every **30 s** while any pool moved in the last 10 min;
- every 120 s while any intent is open;
- every 600 s otherwise (the runner's quiet cadence [R, `launch-day1-solana.md` §6]).

The runner calls `tick()`. A canister timer would remove the runner from the latency path (§13.1).

**Rebalancing beyond arbitrage:**

- Desk wX above 60 % of the position's value → burn the excess.
- SOL above 60 % → add one-sided liquidity.
- Re-range when the price passes 70 % of the way to a range edge.

The first two are rare (daily check). **Re-centring is routine** with the narrow range: 70 % of the way to an edge of
`[p/1.21, 1.21p]` is a floor move of ≈ 14 %.

**But in DAMM v2 the range belongs to the pool, not the position** [R: cp_amm 0.2.4 IDL; `Pool.sqrt_min_price` and
`sqrt_max_price`, and `Position` has no range]. Every position shares the pool's range, and the customizable pool
(`["cpool", max, min]`) is one per pair. A new position cannot move the range, so re-centring means a **new pool**:

1. withdraw and close the old position: `remove_all_liquidity`, `claim_position_fee`, `close_position` (its rent returns; the
   empty pool and its vaults stay);
2. open a new pool around the floor with a fresh `K_nft[n]`. The only DAMM v2 path that lets the desk choose the range of a
   second pool of the same pair is `initialize_pool_with_dynamic_config`:
   - its pool is `["pool", config, max, min]`, one per (config, pair);
   - the config's `pool_creator_authority` (here `K_desk[i]`) signs;
   - only Meteora's operator can create dynamic configs.
3. So each re-centre spends one **dynamic config issued by Meteora for `K_desk`**. A set of N configs gives every collection N
   re-centres (the builder exists, `solix2::damm::initialize_pool_with_dynamic_config`, checked against the IDL client; not
   landed, since no such config exists).

Each re-centre also costs ≈ 0.03 SOL of new pool rent (desk money) and one more threshold signature (≈ $0.035).

**Open item S4 (before W7):** ask Meteora for dynamic configs naming `K_desk` (or one shared desk key), or choose a fallback:

- **a wide range, never re-centred**: `[p/4, 4p]` at ≈ 10 SOL a side also passes both Jupiter tests [M, spikes §3.3], at
  ≈ 2.5–3 % on a 0.5 SOL buy [A, half of the 5.5 % measured at 5 SOL];
- **another venue for the desk's range**: Meteora DLMM, whose positions carry their own bins [W, not spiked].

Until S4 is settled, W7 builds with one customizable pool per collection, and a range that the floor leaves is treated as a
stranded pool (the arb stops at its edge and the desk withdraws).

**This is a market maker run by code.** Its profit and loss is the desk's, never the module's: mint and burn are book-neutral
(`intake` and `release`) [R, `wx-pools-design.md` §7]. The pool's LVR is paid by the desk. The site states that plainly.

---

## 6. Direct mint and burn with the canister (item 5)

The deep, asynchronous route. It works without any pool, for any size up to the caps, from any wallet or exchange withdrawal.

### 6.1 Intents and deposit addresses

**Choice: per-intent deposit addresses**, not memo-tagged deposits.

- Memos need a site-built transaction, and exchanges and many wallets cannot add one.
- Memo detection needs `getSignaturesForAddress` plus a `getTransaction` per deposit, each agreed by two providers (the
  existing `relaysol` parsers do that for the canister's own address [R]).
- Deposits mixed into `M[i]` would blur the books (§4.4).

Per-intent addresses keep user money apart until it is processed, and need **one** `getMultipleAccounts` per tick for up to 100
open intents. The cost is one extra signature per operation (≈ $0.035), covered by the intent fee (O-8).

Update calls (anonymous callers allowed; rate-limited per caller and per destination; at most 300 open intents in all;
idempotent):

| Call | Returns | The user then sends | The canister then |
|---|---|---|---|
| `open_mint(coll, recipient, max_price?)` | address `A = K_int[n]`, quote, expiry (24 h of no activity) | SOL to `A` (any wallet; repeatable) | per finalized balance increase ≥ the minimum: mint at the current price (refused above `max_price`). **Transaction:** `A` → `M[i]` (lamports less fee), `MintTo(recipient ATA)` (ATA created idempotently, its rent charged to the deposit), Memo. **Signers:** `K_main`, `K_int[n]`, `K_coll[i]` |
| `open_burn(coll, sol_recipient, min_price?)` | address `A`; the user's transfer creates `A`'s wX ATA (the site does it; any wallet can) | wX to `A`'s ATA | burn at the current price. **Transaction:** `Burn(A's ATA)`, `M[i]` → `sol_recipient` (paid less fee), Memo. **Signers:** `K_main`, `K_int[n]`, `K_coll[i]`. Then, as a transaction of its own, `CloseAccount(A's ATA → K_main)` once finalized shows it empty (audit B1-04: a stranger's dust can then fail only the close; the rent pays for that transaction) |
| `open_redeem_coin(coin, wx_recipient)` | address `A` | coins to `A`'s coin ATA | §7 |
| `open_redeem_nft(coll, destination)` | address `A`; `destination` checked at open (Ethereum address; taproot on the registry's network; later Solana) | 1 wX (or n) to `A`'s ATA | §8 |

Rules:

- **Minimum deposit 0.05 SOL** (mint) or 0.01 wX (burn) [A]. Anything below waits; at expiry it is swept to the treasury if it
  covers the fee, otherwise left alone.
- **Refunds.** A deposit is refunded whole, less the fee, if:
  - the operation is refused (stale floor, poster paused, an hourly bucket or cap, a price bound, a paused collection);
  - or it is still unprocessable after 6 h.

  The refund goes to `recipient` (mint) or to the wX sender's owner read from the ATA's history (burn; the transfer that funded
  it is read with `getTransaction`, agreed by both providers). The site always asks for `recipient` = the connected wallet. An
  intent whose refund cannot be worked out pages the operator; it does not guess.
- **Partial fills.** A mint over the hourly bucket mints what fits and keeps the rest for the next hour, refunding it after 6 h.
- **Idempotency.** `processed_balance` per intent moves only on finalization (§4.4 rule 1). A balance rise during processing is
  the next operation.
- **Latency.** Finalization of the deposit (≈ 13 s) + the next tick (30–120 s while intents are open) + 3 threshold signatures
  in parallel (≈ 1.2 s on a local replica, S3; mainnet unmeasured) + finalization (≈ 13 s): **≈ 30–150 s** [A]. The site shows each step from the
  `intent(n)` query.
- **Fee.** Proposed 0.0015 SOL per operation (O-8). It covers 3 signatures (≈ 80 B cycles ≈ $0.11) plus 15,000 lamports of
  Solana fees plus a priority fee. The spread already goes to the module, as on chain.

### 6.2 Spam and abuse

- Opening intents costs the caller nothing, and the canister ≈ a few M cycles. Limits: 200 open intents that never saw a
  deposit, 300 that did (audit B1-05: free openings never take a funded slot), 5 per caller principal per hour, 3 open per
  recipient. A never-funded intent leaves the tick's batched read an hour after it expires; for the rest of its grace it is read
  in a rotation of one `getMultipleAccounts` a tick.
- Dust to many intents is ignored below the minimum.
- A 1-lamport deposit cannot make the canister sign: only a balance ≥ the minimum triggers work.

---

## 7. Per-coin backing and its redemption (item 6)

- **Book `CoinVault`** per graduated coin: coin mint, collection, DBC pool, migrated DAMM pool, `launched_at`, creator at
  backing, `wx_in`, `coins_burned`, `wx_paid`, `backed_at`. The on-chain balance is `K_coin[c]`'s wX ATA. They must agree: a
  balance below the book raises an incident; above it is unbooked (§4.4).
- **Redemption.** A holder sends `a` coins to an `open_redeem_coin` address. At finalized, the canister reads the vault balance
  `V` and the coin mint's live supply `S`, which still includes the `a` coins. It pays `⌊V × a / S⌋` wX in one transaction:
  1. `Burn(a)` from `A`'s coin ATA (`K_int` signs);
  2. `TransferChecked(V-ATA → recipient's wX ATA)` (`K_coin[c]` signs);
  3. `CloseAccount(A's coin ATA)`;
  4. Memo.

  The rounding is down, as rook_backing's `payout_never_overpays` property requires [R, `rook-backing-as-built.md` §3]. That
  test ports. Redemptions of one coin go one at a time on the line, so `V` and `S` are fresh for each.
- **The coins locked in the migrated pool** are never burned, so their share stays in the vault for the coins still out. An
  arbitrageur who buys them from the pool can redeem them.
- **No backing before migration.** A coin that never graduates has no vault and no claim, as before.
- **Trust.** The coin holder's wX is in an address only the canister can move. Nothing on chain stops the canister paying it
  elsewhere. The balance guard plus the published book make a diversion visible within a reconcile cycle; they do not prevent
  it (§10.3).

---

## 8. NFT custody and redemption (item 7)

What the canister does now is unchanged:

- Ethereum NFTs through `ethflow` (Seaport buys, Relay SOL → ETH legs, the secp256k1 custody address);
- Bitcoin inscriptions through `frogflow` and `convflow` (Ordinals Wallet and UniSat buys, Relay SOL → BTC, BIP340 custody);
- custody caps (`custodycap.rs`), the outbound limiter and the A4-L-1 exit rule.

What changes is that **every former rook_core step is now a book update**, and most need **no Solana transaction**:

| Before (rook_core instruction the canister signed) | Canister-only |
|---|---|
| `advance(amount)` | book: `Module::advance`; then one Solana transfer `M[i]` → `K_relay` (or the Relay deposit straight from `M[i]`) for the purchase leg |
| `mint_item` (1 wX to the module's account) + `settle_advance` (burn it) | book only: `held += 1`, `eth_backed −= 1`, item recorded with its custody proof. **No wX moves** (net supply unchanged, as on chain) |
| `return_advance(lamports)` | book + the Relay refund landing at `M[i]` |
| `item_arrived(cost)` | book + transfer `M[i]` → custody leg |
| `redeem(destination)` | the user sends 1 wX to an `open_redeem_nft` address; transaction `Burn(1 wX)` + Memo; ticket created; drand round pinned |
| `prove_round` + `resolve` | internal: two relays agree **and the canister verifies the BLS signature** (`drand.rs`, already does [R, `solana-migration.md` §2.9 D-i]) |
| `note_sent` / `record_delivery` / `mark_undeliverable` | book; delivery through the existing outbound flows |
| `refund` | **re-mint 1 wX** to the ticket owner (MintTo, `K_coll[i]`), Memo `refund:<ticket>` |

**1 wX → 1 real NFT** holds as before. Redemption needs `held ≥ open_tickets + 1`. While a collection's float is mostly
SOL-backed (§3.2), redemptions wait for purchases, and the site shows `held − open tickets` per collection.

**Solana NFTs** (`ListingKind::SolNft`, custody `K_main`): built as `src/solnftflow/` on these books (floor, the advance + purchase in one judged transaction, custody, delivery); see `solana-nfts-canister.md` §9.

---

## 9. Treasury, raffle, fueler, tiers, keeper (item 8)

### 9.1 Treasury

Income arrives in **wX of 26 collections** (curve and LP fees), plus SOL (pool fees, module `skim`). Booking:

1. Fee wX of collection i is split:
   - `q_pool` goes to `K_desk[i]` while its pool is under target (§5.3);
   - the rest is **cashed**: burned at module i through the public burn, with the parked D-j `cash_fees` bound kept (it waits
     if the burn pays under 95 % of the accepted floor [R, branch `solana-wx-paired-wip` `constants.rs`]).

   The SOL goes to `K_tre`. Cashing raises nobody's backing and lowers nobody's. It is an ordinary burn.
2. `allocate`: 90 % reserves, 10 % ops (shares changed only after the 24 h delay).
3. `distribute`, daily: each collection's share by 7-day volume. It goes to `surplus` up to the **reserve cap** (`reserve_cap_bps
   × accepted`, 20,000 by default). The overflow is split `raffle_bps` (5,000) to the collection's raffle pot and the rest to
   ops [R, `rook-core-as-built.md` §9.10–9.11].

   SOL moves `K_tre` → `M[i]` (surplus) in one transaction for all collections whose share is ≥ 0.01 SOL. The books move at
   finalization.

### 9.2 Ops and the fueler

Ops SOL goes from `K_tre` to the **fueler's hot key**, by a canister transfer whenever ops ≥ 0.1 SOL. The fueler's
`allocateIx` and `decodeTreasury` (`ops/src/fuel/sol.ts`) are replaced by "watch for the canister's transfer" [R, reuse map].
The Relay SOL → ETH → ckETH → ICP → cycles legs are unchanged. The `FUELER_*` caps stay.

### 9.3 Raffle

Weekly per collection, as `raffle-design.md` §5. Everything is internal except:

- `fund_prize`: the pot buys one whole wX through the public mint, held at a `K_tre` wX ATA;
- the claim: `TransferChecked` to the winner's ATA, the canister paying its rent.

The snapshot rules stay (`raffle.rs`, published before commit, `raffle_snapshot` query) with two changes:

- `wx(c)` = the `K_coin[c]` ATA balance;
- the exclusions add **every canister-derived address** (`K_fc`, `K_coin[*]`, `K_desk[*]`, `K_int[*]`, `K_tre`), since they are
  on-curve.

The pinned drand round is 20 rounds after the commit, and the BLS signature is verified by the canister.

**Trust note.** On chain, `claim_raffle` checked a Merkle proof against a committed root. Now the canister checks its own
tree. Publication before the draw is what keeps it honest, as before.

### 9.4 Tiers and keeper

- Tiers become `T_wX` per collection (§3.2). A later move is a new DBC config with `fee_claimer = K_fc`. The 8-configs limit
  per partner was rook_backing's; the canister's limit is a constant.
- "Keeper" no longer exists as a role. Floor posts, cranks, draws and distributions are all canister work on the `solcrank`
  framework (one crank per tick, ≤ 48 a day, cheap reads first) [R].

---

## 10. Verifiability, trust, governance (item 9)

### 10.1 Public queries

Every one is a query call (free, certified when served through `http_request` with certified data [W]):

- `books(coll)`: params, floor (accepted, target, posted_at, confidence, depth), spreads, mint and burn price now, `backing`,
  `reserve`, `surplus`, `revenue`, reserve units, advance in transit, buckets, 7-day volume, `M[i]` and its last finalized
  balance.
- `ledger(coll)`: `held`, `eth_backed`, open tickets, `expected_supply = (held − open_tickets)·10⁹ + eth_backed` (the rook_core
  wrapper ledger [R]), the last finalized on-chain supply, and every item with its custody proof (Ethereum: contract, token id,
  custody address; Bitcoin: inscription id, outpoint).
- `coin_vaults(coll)`, `coin_vault(coin)`: the book and the ATA address.
- `treasury()`, `desk(coll)` (position, inventory, P&L, arbs), `raffle(coll)` + `raffle_snapshot(coll, round)`.
- `ledger_log(from, n)`: every book entry with its Solana signature and memo id. Anyone can replay it.
- `intent(n)`: state and transactions.
- `keys()`: every derived address and its role (§1.1).

### 10.2 Proof of reserves (reconciler + site page)

`ops/src/reconcile.ts` keeps its multi-provider reader and drops the rook_core decoders [R, reuse map]. Every 10 min, and on
demand, it checks per collection:

1. Solana, finalized:
   - the wX[i] mint supply equals `ledger.expected_supply` (± in-flight entries named by memo);
   - mint authority = `K_coll[i]`; freeze authority none;
   - `balance(M[i]) − rent ≥ books(i)`;
   - each coin ATA ≥ its book;
   - the desk position and inventory match `desk(i)`.
2. Ethereum: `ownerOf(token) == custody` for every held item. Bitcoin: each inscription at the custody taproot (ord API).
3. Cross-checks: the replayed memos from Solana history equal `ledger_log`, and there is no `MintTo` of wX[i] without a ledger
   entry. Any unexplained mint is **SEV1**.

It publishes a signed JSON. The site's **Reserves** page shows, per collection: wX supply = NFTs in custody (links) + SOL-backed
units, SOL in `M[i]` vs books, coin vault balances, and the last check time.

Limit: the reconciler is the operator's service. Anyone can run it (open source, public RPC), and the page says so.

### 10.3 Monitor alerts (`monitor/`)

- Kept: `solLineAlerts` (generic) and `raffleAlerts`.
- Rewritten: `crankAlerts` kinds (graduation backing, fee claims, distribution, arbs).

New alerts:

| Alert | Severity |
|---|---|
| supply ≠ ledger, or an unknown `MintTo` | SEV1 |
| a balance under its book | SEV1 |
| a timelock proposal open | SEV2 + public post |
| cycles < 30 days of burn | SEV2; < 10 days SEV1 |
| an intent unprocessed > 15 min | SEV3 |
| pool outside the band > 1 h while mint is open | SEV3 |
| no tick for 30 min | SEV2 |
| a DBC or DAMM v2 ProgramData change | SEV2 (Meteora upgrade; pin the hash, as `rook-backing-as-built.md` §6 asks) |

### 10.4 Governance

- **Controller:** the timelock canister (`autobuyer/timelock-rs`) is the **only** controller. An upgrade is proposed with its
  wasm hash, waits **72 h** in public, can be cancelled by k = 2 of the guardians g1–g3, and is then executed by anyone
  [R, `docs/v2/timelock-controller.md`, `ref-robinhood-contracts.md` §387]. Proposers p1 and p2 are the operator's (decision
  O-7: name the guardians, ideally people outside the operator).
- **Builds:** reproducible (Docker, `--remap-path-prefix`), with the hash published with the proposal. The monitor watches the
  timelock (`scripts/timelock-watch.mjs` [R]).
- **Guardian methods in the canister**, stop-only, effective at once:
  - `pause_collection(i)`: mint and intents closed; burns stay open by the module's own rules; NFT redemptions keep running;
  - `pause_desk(i)`;
  - `pause_all_mints`.

  Unpausing takes a 24 h canister-enforced delay. **No method moves funds by hand.**
- **What 72 h means:** a malicious or broken upgrade is visible 72 h ahead. Holders can burn wX for SOL and redeem coins in that
  window. The mint and burn caps (§4.3) bound how fast that can happen. **The caps also slow an honest exit.**

### 10.5 Trust comparison (honest)

| | Solana programs (as they would have launched, D-g) | Canister-only (this design) | Robinhood (FloorModule contracts, paused) |
|---|---|---|---|
| Who enforces mint, burn and books | Solana validators running rook_core | the canister's code, on one IC subnet | Robinhood Chain (an Arbitrum Orbit L2, one sequencer) |
| Can the operator change the rules | yes, at once: the upgrade authority was one fund wallet until a later Squads move | yes, after a **public 72 h** delay with a guardian veto | no (non-upgradeable contracts; Safe + 7-day delays on parameters) |
| Unbacked wX mint by a code bug | possible (program bug) | possible (canister bug); the supply guard and reconciler catch it at signing or within 10 min | possible (contract bug) |
| A compromised executor | the canister could misuse only custodian and poster powers inside program limits (advance caps, 3 items a day, clamps) | **≥ 1/3 of the canister's subnet nodes** (5 of 13; 12 of 34 on a fiduciary subnet) could make it sign anything: mint wX, empty `M[i]` | the custodian canister, bounded by contract limits |
| Key that holds the money | program PDAs (no key) | threshold Ed25519 key on the IC key subnet (no single holder) | contract balances (no key) |
| Liveness | the programs keep working without the canister (burns, redeem_backing) | **subnet down = frozen issuer**: no mint, burn, coin redemption or NFT redemption; coins and wX still trade on Meteora | the sequencer is a single point of liveness |
| Deposit needed | ≈ 12 SOL program rent (refused) | ≈ 0.6 SOL + desk money | small |
| Volume | Solana | Solana | none (the reason it stopped) |

**One-line version for the site:** *"Rook's floor module runs inside an Internet Computer canister, not a Solana program. Its
code can change only after a public 72-hour delay, everything it holds is published and checked against Solana, Ethereum and
Bitcoin every 10 minutes, and if the canister stops, your wX and coins keep trading but cannot be minted, burned or redeemed
until it is back."*

---

## 11. Reuse map (item 10)

Inventory of 2026-10-10 [R]. Line counts are approximate.

**The rule:** everything built on Meteora DBC and DAMM v2, and the canister's generic Solana stack, survives. Everything that
decodes rook_core or rook_backing accounts, derives their PDAs or builds their instructions goes.

| Component | Path (≈ lines) | Verdict | What changes |
|---|---|---|---|
| Canister: generic Solana | `soltx` 741, `solrpc` 742, `solsend` 535, `solkey` 147, `relaysol` 575, `drand` 423 | **keep** | the SPL, Memo, Metaplex, DBC and DAMM builders are built on `soltx` in the new `solix2` (W2, done); `solkey` gains paths and multi-signer messages; `solsend` lines become N (W3) |
| Canister: Meteora decoders | `solread` (DBC pool, DAMM pool and position, ≈ 300 of 1,077) | **superseded** | `solix2::decode` has the full DBC `PoolConfig` / `VirtualPool` and DAMM v2 `Pool` / `Position` decoders, plus SPL `Mint` and token `Account` |
| Canister: rook_core/rook_backing | `solix` 276, rest of `solread` ≈ 700, `solcrank` builders ≈ 600 of 1,511, most of `frogsol` 627, `Home::Solana` fields (`rook_core`, `rook_backing`) in `canister.rs`, ≈ 27 refs in `ethflow` | **delete or rewrite** | replaced by the ported books (W1) and the book-plus-transfer steps (W8); `solcrank`'s pass, budget and interval framework is kept |
| Canister: `solhome` | 1,181 | **adapt** | `SolLine`, `LineStep`, `SolIo`, `SolNet` and the FX helpers stay; `SolHomeCfg`, `rook()`, `sink_view` and `wx_supply` become internal |
| Canister: `raffle.rs` | 395 | **keep, adapt input** | `wx(c)` from `K_coin[c]` ATAs; extra exclusions; phases move in from rook_core |
| rook_core logic and tests | `solana/programs/rook_core/src/state/*`, `tests/suite/*` | **port** | §4.1. The programs themselves are never deployed. `solana/` stays in the repository as the reference and for the test vectors |
| rook_backing | `instructions/back.rs`, `sweep.rs`, `dbc.rs`; branch `solana-wx-paired-wip` | **reference only** | DBC account lists and the wX-quote `back` / `cash_fees` logic inform W6 and §9.1 |
| Site: generic | `web-rook/src/rook/sol/{base58,borsh,tx,rpc,send,wallet,pda,keys,format,offchain,upload,floors,tracked,collectionLinks}.ts` | **keep** | — |
| Site: terminal + launch wizard | `dbc.ts` 79, `dbcState.ts` 174, `trade.ts` 61, `board.ts` 70, `market.ts`, `launchWizard.ts` 201, `graduation.ts`, `pages/SolLaunch.tsx` 353, `SolCoins.tsx`, `ui/{market,Board,SolChart}.tsx` | **keep** | quote token = the collection's wX (prices in wX with a SOL/USD view); buys route SOL → wX → coin (W12) |
| Site: backing, raffle, fund | `backing.ts` 46, `raffle.ts` 164 (+ hooks, `RafflePanel`, `Raffles`), `fund.ts` 58, `SolCoin.tsx` backing panel, `SolCollections.tsx` pills, `siteConfig.ts`, `config/solana.ts` | **adapt** | data from canister queries; claimer = `K_fc`; no listing PDAs; the deploy-fund page goes; new intent UI and Reserves page |
| Site: demo + docs | `demo*.ts` ≈ 530; `content/docs/*.md` 656 | **adapt / rewrite** | backing, how-it-works and risks rewritten for §10.5; `06-deploy-fund` removed |
| Indexer | `indexer/src/sol/*` 1,554 | **keep** | no program dependency; add wX-quoted candles via the wX/SOL pool, and memo-tagged canister events (W13); the DBC and DAMM IDL path points into `solana/programs/rook_backing/idls/` |
| Runner, Ordinals floor | `ops/src/runner.ts` 401, `solfloor.ts` 112, `relaysol.ts` | **keep** | the floor feeds the internal `post_floor`; multi-collection Ordinals floors are O-6 |
| Reconciler | `ops/src/reconcile.ts` 354, `solread.ts` 241, `solreserves.ts` 171, `backing.ts` 108 | **rewrite the Solana branch** | the multi-provider reader and backing arithmetic stay; checks as §10.2 |
| Fueler | `ops/src/fueler.ts` + `fuel/` (`sol.ts` 325) | **adapt** | ≈ 100 lines: no `Treasury.ops` / `allocate`; the canister pays the hot key; Relay legs unchanged |
| Ceremony + launch | `launch-params.mjs` 458, `day1-pins.json`, `ceremony-day1.sh`, `site-config.mjs`; `rook-ceremony` (`ceremony*.rs` ≈ 2.9k) | **adapt** | keep the DBC `create_config` builder, the checks, the pins and `listing_order`; add `mints`; drop the program-key reservation and `init` / `list` / owner subcommands; claimer = `K_fc`; caps value-based |
| Deploy-fund tooling | `deploy-later.sh`, `fund-status.mjs`, `accept-owner.html`, `rook-devnet` (≈ 90 % program-bound) | **obsolete** | `rook-devnet`'s `dbcx::create_config` and its launch → threshold → migrate skeleton are reused in S1 |
| Monitor | `monitor/src/ethnft.mjs` 906, `candid.mjs`, `argdiff.mjs`, `fueler.mjs`, `config.mjs` | **keep / adapt** | `solLineAlerts` and `raffleAlerts` kept; `crankAlerts` kinds and `argdiff` `Home::Solana` fields adapted; §10.3 alerts added |
| Timelock | `autobuyer/timelock-rs`, `scripts/timelock-watch.mjs` | **keep** | the chunked install path (W14) |
| Ethereum and Bitcoin sides | `ethflow`, `ethnft`, `frogflow`, `convflow`, `custody*`, `outbound`, `psbt`, `seaport`, `relayconv`, `relayeth` | **keep** | only their home steps change (§8) |

## 12. Day-1 cost and sequence (item 11)

### 12.1 Sequence

1. **Spikes S1–S3 pass** (§15). Nothing below happens before.
2. **Canister:** create it on the chosen subnet (O-10), install it with `home = Solana { cluster, rpc_urls, nonce_seeds[],
   collections[] }` behind the timelock (controller handed over at once), and top up its cycles (§12.3). Read `keys()`: every
   `K_*` address.
3. **Fund `K_main`** with the operating float (≈ 0.5 SOL). The canister opens its nonce accounts (one per line, 0.00145 SOL
   each; 4 lines, §13.1).
4. **Ceremony part 1, wX:** `rook-ceremony mints` creates the 26 mints and metadata and hands authority to `K_coll[i]` (§2).
   Read back, pinned.
5. **Ceremony part 2, configs:** `rook-ceremony day1` with `quote_mint = wX[i]`, `fee_claimer = leftover_receiver = K_fc`, one
   curve per collection (§3.2), plus ROOK (SOL-paired, unchanged). Simulated, reviewed (`review.reviewed_by`), then sent. The
   canister then checks every pinned config itself and refuses any mismatch.
6. **Floors:** the canister starts posting each collection's floor from its readings (12 collections have feeds today, §0).
   Mint opens per collection when its floor is fresh.
7. **Pools** (per O-1, revised): none at launch, or a tiny anti-squat pool. Later, for each collection whose desk has
   collected enough fee money:
   - create the desk's two ATAs;
   - mint the seed wX + wrap + create the pool, in one transaction that fits a durable nonce (§5.1);
   - then the memo;
   - grow the pool from fees toward the target.
8. **Site** switches to the live config; coins can launch.

### 12.2 Day-1 SOL

| Item | Count | SOL each | SOL |
|---|---|---|---|
| wX mint + Metaplex metadata (rent + 0.01 fee) [M] | 26 | 0.0165772 | 0.4310 |
| DBC `create_config` (rent + 2 signatures) [M] | 26 | 0.00598408 | 0.1556 |
| ROOK config + pool + `transfer_pool_creator` [M] | 1 | 0.02657572 | 0.0266 |
| Ceremony transaction fees (≈ 30) | | | ≈ 0.0005 |
| **Fixed day-1 total** | | | **≈ 0.614** |
| Canister operating float (nonces, ATAs, fees, a month at launch volumes [M, `launch-day1-solana.md` §7]) | | | 0.5 |
| wX/SOL pool rent (DAMM v2 pool, vaults, position, NFT) [M, `wx-pools-design.md` §9]: **desk money, paid by `K_desk[i]`** (spikes §2.3), from fees | per pool | ≈ 0.03 | 0 at launch (revised O-1) |
| Pool depth (O-1, revised) | | 2 × S per pool, from fees; target S ≈ 10 SOL | 0 at launch |

**So:** ≈ **1.1 SOL with no pools** for all 26 (§16 cut launch to ≈ 0.2 SOL for five collections). The old options b and c
(≈ 7.3 and ≈ 14.9 SOL of seeds) are withdrawn: their pools were ≈ 20× too thin to help, and they would fail Jupiter's
post-grace test (spikes §2.4, §3.3). Pool depth now comes from fees, ≈ 20 SOL of desk money per collection at target, and is
recoverable (§5.3).

Per graduation and per user: 0.00204 SOL for the coin vault ATA (the canister pays it out of the treasury). Recipients' ATAs
are paid by the deposit. Raffle prize ATAs: 0.00204 once per collection.

### 12.3 Cycles

1 T cycles = 1 XDR ≈ $1.354. A signature ≈ 26.15 B [M]. Reads are as `launch-day1-solana.md` §6 measured [M]: a quiet tick
≈ 2.5 B of Solana outcalls (≈ 5.5 B with Ethereum and Bitcoin); 0.55–0.81 T/day at rest with no active collection.

| Load [A] | Signatures/day | Reads/day | ≈ T/day | ≈ $/day |
|---|---|---|---|---|
| Quiet (no intents; 600 s ticks; 5 cranks) | ≈ 10 | 0.6–0.8 T | ≈ 0.9 | ≈ 1.2 |
| Launch week (50 intents, 30 arbs, 20 cranks, 30 s ticks for 6 h/day) | ≈ 250 | ≈ 2 T | ≈ 8.5 | ≈ 11.5 |
| Busy (500 intents, 200 arbs, 48 cranks) | ≈ 2,000 | ≈ 4 T | ≈ 56 | ≈ 76 |

At the busy load, intent fees (0.0015 SOL × 500 ≈ $80 at $107/SOL) cover intents. Arbs pay for themselves by the profit floor
(§5.4).

On a 34-node fiduciary subnet, execution and outcall costs are ≈ 2.6× [W]; signature cost does not change.

**Reserve:** keep ≥ 30 days of the launch-week rate (≈ 255 T, ≈ $345). The `freezing_threshold` is 90 days. The IC has 3.3 T
today [R, pause doc §3], which is not enough. The fueler (10 % ops share) is the long-run source [R, `fueler-solana.md`].

**Deletion is permanent loss.** A canister that runs out of cycles is frozen and then **deleted**. Its derived keys die with it:
every `M[i]`, every coin vault, every config's fee claimer, every wX mint authority. The cycles alarm (§10.3) is therefore
SEV1 at 10 days.

---

## 13. Effort, risks, audit, decisions (item 12)

### 13.1 Effort (engineer-days at this project's pace; one person; tests included)

| # | Workstream | Days | Reuses |
|---|---|---|---|
| W1 | Port the floor module, wrapper ledger, treasury and raffle books into canister modules, with their tests (props, vectors, audit suites) | 5–7 | rook_core `state/*`, tests |
| W2 | SPL Token / Memo / Metaplex builders; DBC (`withdraw_migration_fee`, `claim_trading_fee`, `withdraw_leftover`, `PoolConfig` decoder) and DAMM v2 (`initialize_customizable_pool`, `add/remove_liquidity`, `swap`, `claim_position_fee`) builders with CLI and mainnet byte fixtures. **Done 2026-10-10:** `src/solix2` (`spl`, `mpl`, `dbc`, `damm`, `decode`, `guard`, `route`); 42 landed instructions + 2 landed bundles + 1 IDL-only instruction + 11 accounts in `tests/fixtures/solix2/flow.json` (written by `solana/tests/solix2_fixtures.rs`); `tests/solix2_builders.rs` (14 tests) | 4–5 | `soltx`, site `dbc.ts`, spike q1/q2 account lists, `solread` DBC/DAMM decoders |
| W3 | Multi-key signing (derived paths, multi-signer messages, the `sign_message` rule change) and **multi-line send** (4 lines: custody, intents, desk, cranks; today there is ONE singleton line [R]); upgrade-tuple migration | 3–4 | `solkey`, `solsend`, `solhome::SolLine` |
| W4 | Seams: book-first operations, supply and balance guards, memo ledger, unbooked sweeps (§4.4) | 3–4 | `state.rs` pattern |
| W5 | Intent desk: open/watch/execute/refund for mint, burn, coin redeem and NFT redeem; rate limits; queries | 5–6 | `deposits.rs` patterns, `relaysol` parsers |
| W6 | Coins: config check, `CoinVault`, graduation backing, fee claims, leftovers, coin redemption (replaces solcrank's rook_backing cranks) | 3–4 | `solcrank` framework, rook_backing tests (payout property) |
| W7 | Pool desk: open from fees (the trimmed opening of §5.1), grow toward target, the arb loop, rebalancing, re-centre (a new pool: S4), P&L book | 5–7 | `wx-pools-design.md`, q2 spike, `solix2::route` |
| W8 | Rewire the NFT side: ethflow, frogflow and solhome steps from rook_core instructions to books + SOL transfers (§8); delete `solix` and most of `solread` and `frogsol` | 4–6 | ethflow and frogflow logic unchanged; `SolIo` |
| W9 | Treasury, raffle and fueler loop internal; guardian methods | 2–3 | `raffle.rs`, `fuel/` Relay legs |
| W10 | Ceremony: `rook-ceremony mints` + `day1` with wX quote and `K_fc`; per-collection curves; `launch-params` value-based caps; pins; drop `deploy-later.sh` and the program crates | 3–4 | `rook-ceremony` DBC builder and checks |
| W11 | Reconciler, Reserves page data, monitor alerts | 3–4 | `reconcile.ts` reader, `monitor/` |
| W12 | Site: **its own two-hop buy (SOL → wX → coin, required; spikes §3.5, the canister's `solix2::route::two_hop_buy` is the reference)**, Jupiter as an extra quote where it routes, intent mint/burn/redeem UI with live status, coin and collection backing panels, Reserves page, docs pages rewritten (backing, how it works, risks; the deploy-fund page removed) | 5–7 | terminal, launch wizard, DBC code (kept), raffle UI |
| W13 | Indexer: wX quote prices (candles in wX, SOL and USD through the wX/SOL pool), desk events from memos | 1–2 | `indexer/src/sol` (no program dependency [R]) |
| W14 | Wasm size: the chunked install (D-d) becomes **blocking** (1.79 MB gzipped of 2 MiB already [M]); a canister timer for fast cadence | 1–2 | timelock proposal path |
| W15 | Rehearsal: devnet (DBC, DAMM v2 and Metaplex are deployed there) with an IC `test_key_1` canister, end to end, then a mainnet run of tiny amounts on one collection | 4–5 | `ops/rehearsal/sol-devnet` |
| W16 | Internal audit rounds (seams, intent desk, desk, multi-signer rule, guards) and fixes | 4–6 | `docs/v9/audit-2026-10` method |
| | **Total** | **≈ 56–76**, or ≈ 50–68 if W12 and W16 overlap the rest | |

Spikes (§15) come first: ≈ 3 days, inside the totals.

### 13.2 Risks

1. **Trust concentration (§10.5).** One subnet's integrity and one canister's code hold every collection's money and mint
   authority. Mitigations: the 72 h timelock, stop-only guardians, the guards, the reconciler, caps, and choosing a larger
   subnet (O-10). Residual: real.
2. **Frozen issuer.** A subnet or key-subnet outage stops mint, burn, every redemption, the arb and fee claims. Pools and DBC
   keep trading, and the wX/SOL pool can drift from the floor. IC subnet halts have been rare and short (hours) [W]. Funds are
   safe but frozen. The site shows liveness.
3. **Key and lifetime risk.** Derived keys exist only while this canister exists (§12.3). Cycles exhaustion, a wrong
   `uninstall_code`, or a controller mistake is **total and unrecoverable** loss of every address and every config's fee
   claimer. Mitigations: the timelock as sole controller (it cannot uninstall without 72 h), a large cycles reserve, SEV1 alarms.
4. **Asynchronous UX.** Direct mint and burn take ≈ 40–150 s (§6.1). Buyers who want instant service use the pool, which
   does not exist at launch and is thin until fees build it (§5.3). Early traders may see poor pool prices until the arb catches
   up.
5. **Cycles cost** scales with signatures (≈ $0.035 each). The busy-day rate is ≈ $76/day (§12.3), and must stay covered by
   intent fees, desk profit and the fueler.
6. **Short-NFT float (§3.2).** wX pairing puts every curve's quote reserve into SOL-backed units that the pipeline converts into
   NFTs slowly. A sharp NFT floor rise strands redemptions (burns stay solvent). The worst cases are cheap collections.
7. **Meteora dependencies.**
   - DBC accepts a non-SOL quote mint whose mint authority is a live key: **passed** [M, spike S1]. DBC checks nothing else
     about the quote, so Rook's own checks carry the rest (§2, §3.1).
   - Meteora's operator can disable pools or change their fees [R, `wx-pools-design.md` §8].
   - DBC and DAMM v2 are upgradeable by one key.
   - Re-centring a DAMM v2 range needs dynamic configs that only Meteora issues (§5.4, S4).
8. **Jupiter routing [M, spike S2].** Jupiter routes DBC curves quoted in wX and thin DAMM v2 pools, but only for ≈ 14–30 days
   from each token's creation. After that:
   - wX needs a pool that passes its liquidity test (≈ 10 SOL a side in the narrow range);
   - a coin still on its curve is dropped until it graduates.

   The site's own two-hop transaction is therefore **required**, not a fallback (§5.1). Jupiter's mint-authority indicator
   will show on every wX (informational; it does not block routing).
9. **RPC providers.** Two must agree, or nothing is signed. Higher read volume (intents, pools) may push past free tiers (D-h).
10. **Durable nonces** are slated for removal [W, `solana-migration.md` §2.4]. The `RecentBlockhash` fallback exists [R].
11. **MEV on the desk.** Swaps are bounded by `minimum_amount_out`. A sandwich only makes a desk transaction fail.
12. **Legal and perception.** A canister that issues 26 tokens and holds users' SOL looks more like a custodian than a protocol
    does. This is for the operator's counsel (`docs/legal/`), not decided here.

### 13.3 Audit plan

1. **Internal (W16), before any mainnet money**, in the `audit-2026-10` format:
   - **C1, ported books:** a diff review against rook_core (should be mechanical), plus the ported property tests.
   - **C2, the seams:** book-first ordering, guards, finalization, re-signing, and unbooked funds.
   - **C3, the intent desk:** attribution, refunds, idempotency, spam.
   - **C4, the pool desk:** the no-double-count proof, arb bounds, re-range.
   - **C5, keys:** the multi-signer rule and derivation-path collisions.
   - **C6, Meteora instruction builders:** byte fixtures and simulation.
   - **C7, governance:** guardian powers, the timelock, cycles.
2. **Outside:** one firm with IC canister experience, scoped to C2–C5 and C7 (≈ 10–15 kLoC of new and changed Rust) [A].
   Calendar 3–6 weeks. Cost is the operator's decision (O-9).
3. **Launch gates:** S1–S3 passed; C1–C7 closed; a devnet rehearsal green; a mainnet run with ≤ 1 SOL on one collection for 72 h;
   then caps raised in steps.

---

## 14. Decisions the operator must make

| # | Decision | Proposal |
|---|---|---|
| O-1 | wX/SOL pools: when they open, how deep, who pays (**rewritten after spikes S1–S2**; the old seeds were ≈ 20× too thin and fail Jupiter's post-grace test) | **No seeds; pools grow from fees** (operator go-ahead 2026-10-10). Each collection's `q_pool` share of fee wX builds its own pool, so the busiest collections get depth first. A pool opens at ≈ 0.5 SOL a side + ≈ 0.03 SOL rent (desk money) and grows toward ≈ 10 SOL a side at `[p/1.21, 1.21·p]`, which passes Jupiter's tests. Meanwhile the site's two-hop buy and the direct mint serve buyers. **Open:** whether to open a tiny anti-squat pool with each collection's first mint (≤ 0.1 SOL a side + rent per collection, from the bootstrap float), and S4 (re-centring needs Meteora dynamic configs, §5.4) |
| O-2 | wX/SOL LP: canister-managed and unlocked, or permanently locked | **unlocked, canister-managed** (re-range and recover). Locked = full range, no re-range, seed never back |
| O-3 | Fee wX use: `q_pool` share to the pool desk, the rest cashed to the 90/10 treasury | `q_pool` = 25 % until target depth, then 0 |
| O-4 | Thresholds and caps: `T_wX = max(3, ⌈tier/floor⌉)`, value-based caps (§4.3); cheap collections (Weirdos, RSIC, Runestone; also Punks and Natcats) | list them on day 1 only with the value caps and a visible "mostly SOL-backed" badge, **or** defer them until the multi-listing purchase work ships |
| O-5 | `reserve_bps` with wX quote (25 % of all minted wX never advanced) | keep 2,500 for the first audit; revisit with data |
| O-6 | Day-1 collections: 12 with feeds (11 Ethereum + Frogs); the 14 other Ordinals need the runner's multi-collection floor feed (≈ 2 days, not in the table) to trade at all | day 1 = 12; the Ordinals feed in week 2 |
| O-7 | Governance: who are proposers p1/p2 and guardians g1–g3; guardian powers as §10.4 | guardians include at least one person who is not the operator. **Decided 2026-10-12: no outside guardians; the operator holds every role.** Five distinct keys, all the operator's, each on its own device: p1, p2 (proposers), g1–g3 (guardians, k = 2). The timelock's rules (`timelock-rs/src/rules.rs`) are met unchanged: they require distinct principals, not distinct people. What this keeps: one lost or leaked key neither stops upgrades nor vetoes one. What it gives up: no independent veto; the 72 h delay is a public warning, not a check. The canister's pause guardian = g1. Said plainly on the site (Security, Risks) |
| O-8 | Direct mint and burn fee and minimums | 0.0015 SOL per operation; 0.05 SOL / 0.01 wX minimum |
| O-9 | Outside audit before real money, and budget | yes, scoped as §13.3 |
| O-10 | Canister subnet: a 13-node application subnet, or a 34-node fiduciary subnet (≈ 2.6× compute and outcall cost; integrity needs 12 colluding nodes instead of 5) | **fiduciary**, given that the canister is now the issuer. **Decided 2026-10-12: fiduciary, `pzp6e-ekpqk-3c5x7-2h6so-njoeq-mt45d-h3h6c-q3mxf-vpeq5-fk5o7-yae` (34 nodes, read from the IC dashboard API that day).** Re-read it at the ceremony; the cost is ≈ 2.6× on reads and execution |
| O-11 | The wake desk (requester bonds for dormant collections) | drop for launch (ON in rook_core; nobody needs it until a collection is dormant) |
| O-12 | ROOK | unchanged: SOL-paired, the fund wallet as creator and claimer, no backing |
| O-13 | RPC: stay on free tiers (D-h) or paid from launch | paid (Helius Developer + Ankr), given intent and pool reads. **Decided 2026-10-12: PublicNode (free, keyless) + Helius; Helius Free until it is shown short, then Developer ($49/month) from fees.** On 34 nodes every read reaches each provider 34 times at once, ≈ 2.1 M requests a month at rest. Helius Free (1 M credits, 10 req/s) is likely short on both counts; Ankr Freemium (200 M credits at 500 a Solana request, ≈ 0.4 M requests) is short. The fiduciary devnet run (2026-10-12, PASS) saw no 429 from Helius Free: the burst is served. The monthly cap is still expected to run short; watch it and move to Developer from fees before it does. Both hosts have IPv6 (checked 2026-10-12) |

---

## 15. Spikes before building (≈ 3 days)

**Run 2026-10-10:** S1 PASS, S2 PASS with a time limit, S3 PASS on a local replica (`docs/v9/spikes/s1-s3-canister-only.md`;
corrections at the top of this document). Still open:

- S3 on IC mainnet `test_key_1`: latency, real pricing and queue size;
- how fast Jupiter indexes a new pool, and the exact grace length;
- **S4** (new, found in W2): dynamic configs from Meteora for re-centring the desk's range, or a fallback (§5.4).

The original briefs follow.

- **S1. DBC and DAMM v2 with wX quote and a real-key claimer.**
  - Setup: LiteSVM with the mainnet DBC 0.2.1, DAMM v2 and Metaplex bytes (the q1 harness [R]), and a classic SPL mint with a
    live mint authority as quote.
  - Run: `create_config` (`fee_claimer` = a keypair), launch, buy to threshold in wX, migrate to DAMM v2 MEMECOIN/wX (LP
    locked), `withdraw_migration_fee` into a third-party ATA, `claim_trading_fee`, `claim_position_fee`, `withdraw_leftover`.
  - Pass: everything lands; the fees are in wX.
  - Also: DBC's quote-decimals and curve validation for each `T_wX` in §3.2 (Weirdos 174, RSIC 152).
- **S2. Routing.**
  - Jupiter's quote API against a devnet or mainnet DBC pool and a thin DAMM v2 pool **[W]**.
  - If not routed, measure the site's own two-hop transaction (SOL → wX DAMM v2 swap + DBC swap) for size: one legacy
    transaction, or does it need a lookup table?
- **S3. Canister multi-signer transaction.**
  - On IC mainnet with `test_key_1` (as Q3): one devnet transaction signed by three derived paths (nonce advance,
    `MintTo`, transfer).
  - Measure latency and cycles.
  - Confirm that derived public keys are stable across upgrades.

## 16. Operator decisions, 2026-10-10 and 2026-10-12: fees fund everything; the people choose which collections open

- **Bootstrap paid by the operator:** ROOK's config and launch (≈ 0.027 SOL) plus the float the first openings take
  (wX mint + Metaplex metadata + DBC config ≈ 0.023 SOL each; since 2026-10-12 a collection opens on its first request, below),
  plus a small canister float (≈ 0.05 SOL): **≈ 0.2 SOL in total**. The canister's IC cycles come from the existing ≈ 3.3 T; the fueler tops up from fees.
- **ROOK's fee claimer is the canister's wallet**, so ROOK's fees fund Rook's operations without a manual claim (the earlier
  fund-wallet routing, D-g, applied to the Solana-programs deposit, which no longer exists).
- **Everything else is unlocked by trading volume:** the canister opens the remaining collections one by one, in a fixed unlock
  order, as soon as fees cover each (≈ 0.023 SOL; ≈ 3 SOL of ROOK volume per collection); the canister's running SOL, the
  wX/SOL pool seeds and the cycles reserve are also paid from fees.
- **No launch set: the people choose (operator, 2026-10-12, "we don't select, the people do").** This replaces the "launch five"
  proposed on 2026-10-10 (BAYC, Milady, Pudgy Penguins, Azuki, Bitcoin Frogs) and the five briefly decided on 2026-10-12 (BAYC,
  Milady, Mad Lads, SMB Gen2, Bitcoin Frogs).
  - **The ceremony PINS every collection whose pipeline works:** the 11 Ethereum collections, Bitcoin Frogs (the Bitcoin collection
    whose floor feed and buy pipeline exist) the 12 Solana collections and (since the "no picking" decision, 2026-10-12) the 14 Ordinals: 38 pins (`examples/launch_pins.rs`, `--count 38`, the
    default: `coins::unlock::LAUNCH_COUNT`); the Solana ones are `ListingKind::SolNft` with the canister's own wallet
    (`sol_address`) as custodian (`--sol-custodian`). The three Robinhood Chain collections are pinned after the ceremony's 4663 read-back
    (`--include-robinhood`; the canister refuses to open one before chain 4663 is in `evm_chains`, audit C4-03).
  - **A collection OPENS on demand,** when a creator first asks for it: `coins_request_open(index : nat32) -> (Result)` (anyone, the
    anonymous principal too; idempotent; refused for an index not pinned, open or refused). The requests form a FIFO queue
    (`CoinsState.requested`, `coins_log`'s 4th element); the opening pass takes the oldest first, whatever its index
    (`coins_collections` shows `Requested`, `Opening`, `Open`). With no request pending nothing opens, unless the operator turns on
    `coins_set_auto_open(true)` (default off), which opens the pinned collections in index order for the float left over.
  - **The registry order** (`coins::unlock::ORDER`, a collection's index): Bored Ape Yacht Club, Milady, Mad Lads, SMB Gen2,
    Bitcoin Frogs, Pudgy Penguins, Azuki, Mutant Apes, Doodles, Lil Pudgys, Moonbirds, CloneX, Checks VV, Identity MD, then the ten
    other Solana collections (Claynosaurz, Famous Fox Federation, DeGods, Galactic Geckos, Taiyo Robotics, Okay Bears, SMB Gen3,
    Transdimensional Fox Federation, Pythenians, Cyber Frogs), then the Ordinals as their floor feeds pass the 20 % band gate (OMB
    first). It matters only under the auto-open switch.
  - **The float:** ≈ 0.2 SOL to `K_main` at the ceremony covers about eight openings (≈ 0.023 SOL each, above the 0.05 SOL the money
    flows keep); the rest are paid from fees as they arrive.
  - The site's launch wizard lists every pinned collection; one not open yet says "Not open yet: be the first", its "Open it" calls
    `coins_request_open`, and the launch proceeds once `coins_collections` says `Open` (the launch transaction needs the DBC config).
- **Solana collections:** keys, standards (ten pNFT, two Legacy, no Core, no cNFT), rule sets, floors, tiers (all 85 SOL) and
  `T_wX` (6 to 71) in `deployments/launch/solana-collections.json` (check: `scripts/check-solana-collections.py`); the
  canister side in `docs/v9/solana-nfts-canister.md`.
- **Go-ahead (operator, 2026-10-10, after spikes S1–S3):**
  - "pools grow from fees": no pool seeds at launch, O-1 as rewritten (§5.3). The "wX/SOL pool seeds" above are that
    fee-funded depth;
  - the Solana NFT collections go in the unlock order after the Ethereum collections, as listed above (superseded on 2026-10-12:
    every working collection is pinned and opens on demand).


## 17. S4 result and decision (2026-10-10): wX/SOL pools are Meteora DLMM pairs

Spike S4 (`docs/v9/spikes/s4-wx-pools.md`) found that DAMM v2's price range belongs to the pool, so a canister cannot re-centre it.
**Decision: the per-collection wX/SOL pool is a Meteora DLMM pair, preset 262 (bin step 50, 0.4 % fee, fees in SOL only), one
position owned by the pool-desk key, about 10 SOL a side in a ±10 % band, re-centred in place with `rebalance_liquidity`.** Measured:
0.1 / 0.5 / 1 SOL buys pay +0.81 / +0.80 / +1.03 %; both Jupiter post-grace tests pass; no key to throw away and nothing needed
from Meteora. Rent ≈ 0.16 SOL to open plus ≈ 0.07 SOL per new bin array (not refundable). W7 and the site's first hop use DLMM
`swap2`; new builders in `solix2::dlmm`. Open: re-centre cost at bin step 30, the trimmed size of the past-the-band transaction,
and Jupiter's test on a DLMM pool after its grace period.
