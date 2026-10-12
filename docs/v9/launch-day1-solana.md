# Rook on Solana — day 1 without deploying, and the community-funded deploy (runbook)

Status: **prepared 2026-10-09**, branch `golive-2026-10-08`. Nothing sent to mainnet: every number below was measured by
**simulating** against mainnet-beta (Meteora DBC 0.2.1, rent from the cluster) or by building the programs. Decisions: D-e, D-f, D-g in
`solana-migration.md` §2.9. Tools: `ops/solana/ceremony-day1.sh`, `ops/solana/deploy-later.sh`, `ops/solana/fund-status.mjs`,
`ops/solana/launch-params.mjs` (the launch file), `ops/rehearsal/sol-devnet/tool/src/ceremony.rs` (`rook-ceremony`).

## 1. The idea

Rook's two programs cost ≈ 7–9.5 SOL to deploy (§4), which the operator does not put up (D-f). Day 1 needs none of it:

- **The program ids are reserved, not deployed.** A program id is the public key of a keypair; generating the keypair reserves it.
  Every address Rook needs from the programs is a PDA of those ids and can be derived offline:
  `listing = PDA(["listing", u32 LE id], rook_core)`, `claimer = PDA(["claimer", listing], rook_backing)`,
  `converter = PDA(["converter"], rook_backing)`.
- **The DBC configs are created on day 1** with `fee_claimer = leftover_receiver = claimer` of each collection. Coins launch and trade
  on them at once. The backing share (50 % of each 85 SOL raise, 42.5 SOL) and Rook's partner trading fees **accumulate inside DBC**
  until `rook_backing` exists to claim them.
- **The deploy is community-funded** (D-g): one plain wallet, the operator's own (the **fund wallet**,
  `GGgXnNgtY4ySfz8PLAAwkoh8Ztj7p6jrM9towSaa6Wuy`). People can send SOL to it directly, and a **genesis ROOK coin** on its own DBC config
  pays the fund wallet 80 % of every ROOK curve fee. Once the wallet holds the deploy deposit, `deploy-later.sh` deploys at the
  reserved ids. The upgrade authority and registry owner both go to the fund wallet. The fund wallet is a Phantom / Ledger wallet
  and **never exists as a key file**: during the setup the ceremony payer (a hot key generated on day 1) owns the registry, and the
  fund wallet takes it over at the end with **one signature** (`accept_owner`, §4).

## 2. Verified against DBC 0.2.1's source (`f552f20`, the deployed release) and by simulation

| Question | Answer | Where |
|---|---|---|
| Can `fee_claimer` be the PDA of an undeployed program? | **Yes.** `create_config` takes `fee_claimer` and `leftover_receiver` as `UncheckedAccount` and only stores the keys. `ConfigParameters::validate` does not look at them (except `leftover_receiver != default` for a fixed supply). The account does not need to exist or have an owner. | `ix_create_config.rs`, `process_create_config.rs:646`; **simulated on mainnet for all 12 collections: OK** |
| Can the config's fee claimer ever change? | **No.** `config.fee_claimer` is written once, in `PoolConfig::init` (`state/config.rs:774`). No instruction writes it again. DBC has no close-config instruction. | grep of the whole program |
| Who can claim? | Only a **signer whose key equals `config.fee_claimer`**. That covers `claim_trading_fee`/`2`, `claim_partner_pool_creation_fee`, `partner_withdraw_surplus` (`#[access_control(is_partner_fee_claimer)]`), and `withdraw_migration_fee` (`sender == config.fee_claimer`, `ix_withdraw_migration_fee.rs:102`). A PDA has no private key: only the program **deployed at the reserved id** can sign for it (`invoke_signed`). | `access_control.rs:25` |
| Does anything expire? | **No.** Partner fees are counters in the pool (`partner_quote_fee`), taken with `min(counter, max)` and no time check (`virtual_pool.rs:1091`). The migration fee is gated only by `quote_reserve ≥ threshold` and a one-time status bit. The post-graduation partner LP position (DAMM v2, permanently locked) is minted to `config.fee_claimer` at migration (`migrate_damm_v2_initialize_pool.rs:620`) and its fees accrue in DAMM v2 with no expiry. | |
| Does anything else need the claimer to sign or exist? | **No.** Pool creation records `partner: config.fee_claimer` in an event only. Migration is permissionless (Q1). `withdraw_leftover` is permissionless and pays the leftover receiver's coin account. With pre = post supply **nothing is burned** (DBC can burn at most pre − post = 0, `state/config.rs:931`): the curve's tail (≈ 12.78 coins per collection coin, ≈ 9.03 for ROOK) and any unsold remainder go to the leftover receiver (the claimer PDA; the fund wallet for ROOK). | `ix_initialize_virtual_pool_with_spl_token.rs:209` |
| rook_backing later | `register_partner_config` reads the config (it refuses unless `fee_claimer == claimer`, wSOL, DAMM v2, SPL, fees in SOL, timestamp activation, fee > 0, creator migration share 0; since 2026-10-10 also `leftover_receiver == claimer`, LP permanently locked with no vesting, migrated pool fees in SOL only, no pool creation fee, DBC config version 0). `open_vault` takes `launched_at` from the pool, not from when the vault opens. `back` needs `is_migrated`. So coins that launch and graduate **before** the deploy are backed afterwards, in any order. | `rook-backing-as-built.md` §3, §7.2 |
| ROOK: migration fee 0, creator share at the maximum | DBC allows `fee_percentage = 0` when `creator_fee_percentage = 0` (`MigrationFee::validate`), and `creator_trading_fee_percentage ≤ 100` (`process_create_config.rs:389`). The protocol keeps 20 % (`PROTOCOL_FEE_PERCENT`). At 100 % the creator receives **80 % of every curve fee**. `transfer_pool_creator` is allowed in `PreBondingCurve` with no other condition, so the creator moves to the fund wallet **in the launch transaction itself**. | **simulated on mainnet: OK** |

**One consequence to watch.** The listing ids are pinned on day 1, because each config's claimer derives from `listing_id`.
`rook_core` assigns ids in `list_collection` order from 0. At deploy, the owner **must list exactly in `launch.listing_order`**
(Milady 0 … Identity MD 10, Bitcoin Frogs 11, then NodeMonkes 12, OMB 13, Bitcoin Puppets 14, Bitcoin Punks 15, Natcats 16:
docs/v9/ordinals-collections.md), with nothing listed before. A wrong order points a collection's fees at another
collection's listing. `register_partner_config` refuses the config (`WrongFeeClaimer`) only if the `Partner` was registered under
the listing the config was made for; registered by listing id after a misordered listing, every check passes and that collection's
backing goes to another collection's floor (audit A3-03/A6-02). Since 2026-10-10 `register_partner` checks the rook_core accounts it
pins and emits `PartnerRegistered { listing, listing_id, … }`: the ceremony must check each listing id's name against
`listing_order` before registering configs. **`rook-ceremony list` does this (audit A6-02, §4):** it lists strictly from
`listing_order`, asserts `Registry.listing_count == i` before each `list_collection`, reads every listing field back, and registers
each partner and config **by slug** from `day1-plan.json` (never by listing id, never by discovering configs by `fee_claimer`: anyone
can create look-alike configs, A6-15), refusing any mismatch.

## 3. Day 1

### Risks, stated plainly

- **The two program keypairs (`rook_core-keypair.json`, `rook_backing-keypair.json`) are the keys to every coin's backing.**
  - **Theft:** whoever holds them can deploy *any* program at those ids before Rook does. Such a program can sign as every claimer PDA and withdraw every graduated coin's 42.5 SOL migration fee and all of Rook's partner fees.
  - **Loss:** if the keypairs are lost, nobody can ever deploy there, and those funds stay in DBC forever.
  - Keep two offline copies, never in the repository and never on a shared machine. Delete the working copy after the deploy (a deployed program id cannot be redeployed by anyone).
  - Both scripts refuse a keys directory inside **any** git work tree (the path fully resolved, symlinks included, compared by inode so
    `~/Documents/rook` and `~/Documents/Rook` are the same; audit A6-01), and `.gitignore` ignores `*keypair*.json`,
    `ceremony-payer.json`, `day1/` and `day1-plan.json` as a second layer.
- **Until the deploy, nothing is claimed.** The backing share of graduated coins sits in DBC's quote vaults and is **not** converted to wX. `redeem_backing` does not exist yet. Holders of graduated coins have no NFT floor until `back` runs for their coin after the deploy (anyone can crank it then).
- **Meteora can upgrade DBC** (one key). That is true with or without our programs. `rook_backing` pins DBC 0.2.1's layout, so rerun its tests on any Meteora release.
- **Day 1 also depends on the ceremony payer** (a hot key): it holds about 0.14 SOL for a few minutes. The fund wallet never signs anything on day 1.
- **The configs are permanent and pinned.** `rook-ceremony` refuses a launch file whose `meteora` parameters (the Borsh-encoded
  `ConfigParameters`, every field: fee schedule, curve, sqrt start price, migration fee option, migrated pool fee and its
  `collect_fee_mode = 0`, `pool_creation_fee = 0`, `token_update_authority = 1`, vesting, LP split, supply) are not byte-identical to
  the committed `ops/solana/day1-pins.json` (collection sha256 `8db45a9b…53ae1c`, ROOK `c50f8c33…a5fe96`), or whose `listing_order`
  differs from `deployments/launch/nft-pilot.json` (17 entries: wMILADY … wIDENTITY, wBFROG at 11, then wNODEMONKE, wOMB, wPUPPET,
  wBPUNK, wNATCAT; docs/v9/ordinals-collections.md). `--send` is refused while
  `review.reviewed_by` is null (audit A6-05). The pins change only with `rook-ceremony pins --write` and a reviewed diff.
- **ROOK's metadata is immutable** (`token_update_authority = 1`). Name **ROOK**, symbol **ROOK** (operator decision 2026-10-10),
  uri **`https://res.cloudinary.com/ddwebzwrk/raw/upload/v1791607907/rook-token/rook.json`** (80 bytes, under Metaplex's 200): the
  hosted metadata JSON whose `image` is `https://res.cloudinary.com/ddwebzwrk/image/upload/v1791607907/rook-token/rook.png`.
  **These two Cloudinary files must never be deleted or renamed**: the on-chain uri can never be changed. `--send` refuses an empty
  or placeholder uri and, on mainnet, fetches it and requires name / symbol to match and the image to resolve (audit A6-06). Other
  accepted hosts: `ipfs://`, `https://ipfs.io/ipfs/`, `https://gateway.pinata.cloud/ipfs/`, `https://arweave.net/`.

### What the operator signs on day 1, and the SOL it needs

1. **One plain SOL transfer** from the fund wallet to the ceremony payer the script prints. Phantom "Send" works, or
   `solana transfer --from usb://ledger <payer> 0.14 --allow-unfunded-recipient`. **Amount: 0.139 SOL** (0.128305080 = the 2026-10-09
   measurement + five more configs at 0.005984080, + 0.01 margin). **The fund wallet itself must hold ≥ 0.14 SOL** (it held 0.00089 on
   2026-10-09).
2. Nothing else. The ceremony payer signs the 18 transactions, together with the config and mint keypairs the script generated.
3. Afterwards, return the remainder from the payer with `solana transfer --from <keys>/ceremony-payer.json <fund wallet> ALL`. That
   transfer is signed by the hot key.

### The exact cost (simulated on mainnet-beta, 2026-10-09)

| Transaction | Count | CU | SOL each | SOL |
|---|---|---|---|---|
| DBC `create_config`, one per collection: 1,048-byte config account, rent 0.00597408 (5,080 lamports/byte, SIMD-0194), plus 2 signatures | 17 | 36,967 | 0.005984080 | 0.101729360 |
| ROOK: `create_config` + `initialize_virtual_pool_with_spl_token` (pool, vaults, mint, Metaplex metadata) + `transfer_pool_creator` → fund wallet, in one 1,085-byte transaction, 3 signatures | 1 | 146,990 | 0.026575720 | 0.026575720 |
| **Day 1 total** | 18 | | | **0.128305080** |

The five Ordinals collections added on 2026-10-10 (ids 12-16, docs/v9/ordinals-collections.md) add five configs: **+0.029920400
SOL** on day 1 (measured 0.098384680 for 13 transactions on 2026-10-09; a config's cost is its rent and two signatures, the same
for every collection).

No priority fee is set. Add one with the payer's margin if the cluster is congested. Config rent is **not refundable**: DBC has no
close instruction.

### Commands

```sh
# the launch file (production defaults: cluster mainnet-beta, Bitcoin mainnet; carries `meteora` and `listing_order`)
node ops/solana/launch-params.mjs --out ~/rook-launch/launch.json
# dry run: reserves the ids (keypairs in ~/rook-keys, 0600), prints ids + claimer PDAs, simulates all 18 transactions on mainnet
ops/solana/ceremony-day1.sh --keys ~/rook-keys --launch ~/rook-launch/launch.json --rpc https://api.mainnet-beta.solana.com
# read every line of launch.json, fill in review.reviewed_by / reviewed_at; fund the printed ceremony payer with 0.108 SOL from the
# fund wallet (one transfer), then (the ROOK metadata is immutable: these are the final values, and the defaults):
ops/solana/ceremony-day1.sh --keys ~/rook-keys --launch ~/rook-launch/launch.json --rpc <keyed mainnet RPC> --send \
  --rook-name ROOK --rook-symbol ROOK --rook-uri https://res.cloudinary.com/ddwebzwrk/raw/upload/v1791607907/rook-token/rook.json
```

The script refuses a keys directory inside any git work tree before creating anything. It never overwrites a keypair: a rerun reuses
the ids and configs and checks configs that already exist. It never prints a secret. `--send` repeats the dry run, then asks
`Type 'day1'` after printing the ROOK name / symbol / uri. `<keys>/day1/day1-plan.json` has every id, PDA, unsigned message
(base64), simulation result, the total, the pinned sha256s and `reviewed_by`.

Dry run of 2026-10-09 (throwaway ids, nothing sent): all 12 configs and the ROOK transaction **simulated OK on mainnet**, and the
config DBC would write was decoded with `rook_backing`'s own reader and passed `register_partner_config`'s checks and D-e/D-g's.

## 4. Deploying later: the minimum SOL

Rent on mainnet-beta today is **5,080 lamports per byte** (account size + 128). Loader v3 (upgradeable) needs, per program, a
program account (36 bytes) and a ProgramData account (45 + size; `solana program deploy` sizes it to the binary, `--max-len`
default). While writing it also needs a **buffer** (37 + size), which is refunded when the deploy completes. Deploying `rook_core` first
gives the lower peak. Sizes are SBPF v3 builds of the current source. CU is measured on the LiteSVM suites (SBPF v2 builds of the
same opt-level, all green).

| Build | rook_core | rook_backing | Locked while deployed (returned only by closing) | Peak with a buffer open | + write fees | CU vs O3 | Max CU per ix |
|---|---|---|---|---|---|---|---|
| **opt-level 3 (current: programs `6588b81`, reserved ids, `--locked`, 2026-10-10)** | 889,744 B | 519,288 B | **7.161 SOL** (4.522 + 2.640) | **9.800 SOL** | 0.007 | — | 100,901 (`back`, real convert; measured 2026-10-09) |
| opt-level 3 (2026-10-09, before the audit fixes) | 858,296 B | 502,896 B | 6.918 SOL (4.362 + 2.556) | 9.474 SOL | 0.007 | — | |
| opt-level "s" | 717,576 B | 424,856 B | 5.807 SOL | 7.966 SOL | 0.006 | +15 to +31 % | 118,464 |
| opt-level "z" | 660,840 B | 406,144 B | 5.424 SOL | 7.488 SOL | 0.005 | +50 to +109 % | 146,310 |
| strip | already applied: `cargo build-sbf` emits stripped ELFs (no symbol or debug sections) | | 0 | | | | |

`fund-status.mjs` and `deploy-later.sh` use the current build. They report a deploy target of **9.87 SOL**: peak 9.800 + fees 0.007
+ a 0.06 margin for init and slack. After the deploy, 7.16 SOL stays locked as a deposit. Only `solana program close` returns it, to
the **upgrade authority** (not to contributors), and closing ends the program for good: no claimer can ever sign again, so every
pending and future backing becomes unclaimable, and the id can never be used again. The **listing ceremony** that follows (treasury,
desk, and per listing: listing, wrapper, wX mint + Metaplex metadata, item set, module, module vault, module wX, partner; ≈ 0.93 SOL for the 17 listings,
≈ 0.054 SOL each (≈ 0.66 SOL for the first 12), at 5,080 lamports/byte with Metaplex's 0.01 SOL fee per wX metadata as an upper bound) is a second line in `fund-status.mjs` (audit
A6-11): its `launch_line` is "raised of deploy + listing ceremony". The canister's own Solana costs are not included.
(rook_core's size: 889,744 B is `cargo build-sbf --arch v3` at O3 from `git archive` of `6588b81` with the reserved ids swapped in,
byte-identical on two builds; other build paths give other sizes. `deploy-later.sh` prints the exact one it will deploy.)

- **CU per instruction, O3 → s → z** (LiteSVM): `post_floor` 13,945 → 18,238 → 28,525; `convert` 35,350 → 42,530 → 59,395;
  `mint_item` 41,718 → 52,143 → 65,695; `settle_advance` 32,093 → 38,652 → 55,130; `mint` 36,341 → 43,333 → 60,136;
  `burn` 35,294 → 41,940 → 58,576; `redeem` 26,591 → 32,456 → 41,849; `resolve`(64) 16,770 → 22,065 → 37,821;
  `back` (mock) 84,256 → 93,080 → 104,085; `back` (real) 100,901 → 118,464 → 146,310.
- **All suites stayed green at every level:** rook_core 77, rook_backing 11, real_rook_core 2. Everything stays under 200 k CU.
- **Not adopted.** The audited and rehearsed binary is O3. "s" saves 1.11 SOL of deposit and 1.51 SOL of peak for +15–31 % CU.
  "z" saves 1.49 / 1.99 SOL for up to +109 % CU (more priority fee per transaction, and less headroom on `back` and `resolve`). If
  the fund stalls short of the target, "s" is the reasonable fallback: rebuild, rerun every suite, `--opt-level s`.
- **Loader v4** (one account, no separate buffer) is **not available**. `solana feature status -um` on 2026-10-09 lists SIMD-0167
  "Enable Loader-v4" as `LoaderV4WasAbandoned11111111111111111111111`, **inactive**. Agave 4.1.2's `solana program deploy` uses
  loader v3. Active and relevant: SIMD-0194 (lower rent, 5,080 lamports/byte), SIMD-0431 (loader-v3 minimum extend size), SBPF v3
  (epoch 993).
- **Splitting rook_core.** A "minimal launch set" (registry + floor + wrap + convert) could be a cargo-feature build of the same
  program at the same id: deploy it first, then extend and upgrade it to the full build. Accounts are unchanged and no CPI is added.
  Estimate: treasury and wake are ≈ 20 % of rook_core's instruction code, about 170 kB, so ≈ 0.87 SOL less locked at first.
  - It **does not reduce the total**: the later upgrade pays the extension rent and opens a full-size buffer again (≈ 4.4 SOL transient).
  - Two binaries would need review.
  - The floor module's `revenue` → treasury path would need stubbing.
  - **Not recommended.** The fund target is one-time (≈ 9.9 SOL), and two binaries double the review.

### The deploy, in order (one sitting)

What the operator signs from the fund wallet: **one plain SOL transfer** to the ceremony payer (step 2, Phantom "Send") and **one
transaction**, rook_core `accept_owner` (step 6, Phantom or a Ledger). Nothing else. The fund wallet's key is never exported to a file.
Every other transaction is signed by the ceremony payer `<keys>/ceremony-payer.json`, which owns the registry from `init` until the
fund wallet accepts. **While it does, a hot key controls Rook: do steps 3–7 in one sitting, accept immediately, and delete the file.**

```sh
R=ops/rehearsal/sol-devnet/tool/target/release/rook-ceremony   # deploy-later.sh builds it
NET="--rpc <keyed mainnet RPC> --rpc2 <second provider>"
PLAN=~/rook-keys/day1/day1-plan.json

# 1. build from the reviewed commit/tag + preflight + exact SOL (nothing sent)
ops/solana/deploy-later.sh --keys ~/rook-keys --launch ~/rook-launch/launch.json $NET --ref <reviewed tag>

# 2. FUND WALLET, plain transfer (Phantom "Send"): the printed deploy amount + the listing ceremony (`node ops/solana/fund-status.mjs`,
#    ≈ 0.93 SOL for 17 listings) + 0.02, to the ceremony payer the script prints.

# 3. deploy + init_registry(owner = the ceremony payer) + init_backing + upgrade authorities → the fund wallet
ops/solana/deploy-later.sh --keys ~/rook-keys --launch ~/rook-launch/launch.json $NET --ref <reviewed tag> \
  --priority-fee <micro-lamports per CU> --send

# 4. the listing ceremony, signed and paid by the ceremony payer (dry run first, then --send; Type 'list')
$R list --launch ~/rook-launch/launch.json --plan $PLAN --custody <custody.json> $NET \
  --canister-sol <the canister's sol_address()> --ops <the fueler's hot key address>
$R list --launch ~/rook-launch/launch.json --plan $PLAN --custody <custody.json> $NET \
  --canister-sol <the canister's sol_address()> --ops <the fueler's hot key address> --owner ~/rook-keys/ceremony-payer.json --send

# 5. the ceremony payer proposes the fund wallet as the registry owner (Type 'propose'; refused until step 4 is complete)
$R propose-owner --plan $PLAN $NET --owner ~/rook-keys/ceremony-payer.json --send

# 6. FUND WALLET, ONE signature: accept_owner. Either (a) Phantom (a Ledger connected to Phantom works the same way):
$R accept-owner --plan $PLAN $NET --print-tx          # prints the unsigned tx (base64 + base58) and the page link
python3 -m http.server 8765 --bind 127.0.0.1 --directory ops/solana     # then open the printed http://127.0.0.1:8765/accept-owner.html?tx=…
#    or (b) the Ledger directly (Solana app open; "Blind signing" on, since rook_core is not a program the app knows; Type 'accept'):
$R accept-owner --plan $PLAN $NET --ledger "usb://ledger?key=0" --send

# 7. verify, then make the ceremony payer harmless
$R verify-owner --plan $PLAN $NET                     # must print "verify-owner: OK" (exit 0) BEFORE anything is announced
solana transfer --from ~/rook-keys/ceremony-payer.json GGgXnNgtY4ySfz8PLAAwkoh8Ztj7p6jrM9towSaa6Wuy ALL -u <keyed mainnet RPC>
rm -P ~/rook-keys/ceremony-payer.json                 # and any backup of it
```

- **Step 6 (a), the page** `ops/solana/accept-owner.html` (no external script, no secret): it takes the unsigned transaction from the
  link (`?tx=`) or a paste box (base64 or base58), decodes it and **refuses anything but one rook_core `accept_owner`** whose only
  signer and fee payer is the fund wallet and whose registry is re-derived as `PDA(["registry"], rook_core)`; it shows program,
  registry, signer and the registry's current and pending owner. Check that the program is the reserved rook_core id and the signer
  ends in `…6Wuy`, connect the fund wallet (Wallet Standard: Phantom), press **Sign and send**. It refreshes the blockhash first (the
  printed one is valid ≈ 60 s; nothing is signed until the wallet signs), refuses to send if the wallet changed the message, sends to
  the `rpc` of the link (mainnet: the public endpoint, or `--page-rpc <url>`), waits for confirmation and reads the registry back.
  Wallet extensions do not run on `file://` pages, hence the local `http.server`.
- **Step 6 (b), the Ledger**: `--ledger` takes the solana CLI's locator (`usb://ledger`, `usb://ledger?key=0`, `?key=0/0`, …: the
  account Phantom shows for the Ledger; `solana-keygen pubkey "usb://ledger?key=0"` finds it). The tool refuses a signer other than
  the fund wallet and the pending owner before asking the device. The fund wallet pays the 5,000-lamport fee.
- **Refusals**: `list --owner` with the fund wallet's address (any file holding it) or any key other than day 1's ceremony payer;
  `list --send` once the fund wallet owns the registry; `propose-owner` before the listing ceremony is complete (treasury, desk,
  keeper, all 12 listings; `--allow-incomplete` overrides); `accept-owner` before `propose-owner` or by another key (and rook_core
  refuses `accept_owner` from any key but the pending owner); `accept-owner --keypair` on mainnet-beta's genesis (a stand-in for local
  rehearsals only); `verify-owner` exits 1 until the fund wallet has accepted.
- **`verify-owner`** reads, through both RPCs: `Registry.owner` = the fund wallet and no pending owner; rook_backing's settings pin that
  registry, so its admin (every rook_backing admin instruction reads `Registry.owner`; there is no stored admin) moved with it; both
  upgrade authorities = the fund wallet; the ceremony payer holds no role (owner, keeper, pending keeper, guardian, treasury ops,
  pending ops). The keeper (the canister) and the treasury ops (the fueler) set during `list` are not touched by the handover.
- **If step 6 is delayed**, the ceremony payer still owns the registry and can sign any owner instruction. Rerunning `propose-owner`
  is harmless (it reports "already proposed"); `accept-owner --print-tx` can be rerun for a fresh transaction.

`deploy-later.sh` does the following:
1. **Preflight** (`rook-ceremony preflight`, audit A6-04): the RPC's genesis must be the launch file's cluster (mainnet-beta ⇒
   Bitcoin `Mainnet`, which `init_registry` fixes for good; a `localnet` file is refused on a public cluster); the launch file must be
   the pinned, reviewed one; `<keys>/day1/day1-plan.json` must name these program ids, this ceremony payer and this fund wallet; the
   12 listing / claimer PDAs are recomputed from the keypairs and every day-1 config **on chain** must pay its claimer
   (`fee_claimer = leftover_receiver`, the whole account checked byte by byte), the ROOK config the fund wallet. With `--rpc2` every
   account is read through both providers and must be byte-equal. The fund wallet must be the launch file's and day 1's (A6-08).
2. Builds from **`git archive` of a named commit** (`--ref`; without it HEAD, refused unless `solana/` is clean: no uncommitted or
   untracked file reaches the build; A6-03) in `<keys>/build/<commit>` with `cargo build-sbf --arch v3 -- --locked`, the reserved
   ids swapped in (both `declare_id!`, rook_backing's `ROOK_CORE_ID`, rook_core's `ROOK_BACKING_ID`, `Anchor.toml`; a leftover dev id
   is refused), checks SBPF v3 and the ids, deletes the random keypairs `cargo build-sbf` writes next to the `.so` (A6-14), and
   records the commit and each `.so`'s sha256 in `<keys>/build/<commit>/deploy-record.json`.
3. On a rerun, a program already deployed **with the same bytes** is skipped; one deployed with **other bytes** is refused unless
   `--upgrade` is given.
4. Deploys `rook_core`, then `rook_backing`, with the ceremony payer as upgrade authority, through a buffer keypair kept in the job
   dir (a failed deploy resumes with it, and the CLI prints no seed phrase; any 12-word phrase it prints is redacted, A6-10), with
   `--priority-fee` as `--with-compute-unit-price` (budgeted in the printed SOL at 200,000 CU per write transaction). After each
   deploy, `solana program dump` must equal the built `.so` byte for byte.
5. Runs `rook-ceremony init`: preflight again, then `init_registry(owner = the ceremony payer, the launch file's network, [converter
   PDA])` and `init_backing`, each simulated, sent, decoded and checked (owner = the ceremony payer or, after the handover, the fund
   wallet; network, converter; rook_backing's pinned registry / treasury / converter). Existing accounts are decoded and checked,
   never trusted by existence. The ceremony payer is the registry owner **for the setup only** (steps 4–6 above).
6. **Hands both upgrade authorities to the fund wallet** (part of `--send`; `--keep-authority` opts out and prints the command),
   after `Type 'handover'` showing the wallet's last 6 characters; `solana program show` must then name the fund wallet. The fund
   wallet does not sign this: `solana program set-upgrade-authority … --skip-new-upgrade-authority-signer-check` (without the flag,
   Agave 4.1.2 refuses with "missing signature for supplied pubkey", checked on a local validator). If anything fails after the first
   deploy, the script prints the recovery commands (rerun, hand over, close open buffers).

Rehearsed end to end on a local validator on 2026-10-10 (DBC and Metaplex cloned read-only from mainnet; programs `6588b81`): day 1
sent (13 transactions), deploy + init + handover, the full listing ceremony, then reruns: same bytes skipped, an `--opt-level s` build
refused as an upgrade.

The ownership handover was rehearsed the same way on 2026-10-10 (programs `e53202d`, a stand-in fund wallet keypair): day 1 (13 tx),
deploy + `init_registry(owner = the ceremony payer)` + upgrade authorities → the fund wallet, `list --owner <ceremony payer> --send`
(76 / 76 transactions, keeper and treasury ops set), `propose-owner --send`, `accept-owner` through `accept-owner.html`'s own code with a
Wallet Standard test wallet (blockhash refreshed, the wallet's message checked, sent, registry read back), `verify-owner` → OK; then the
CLI path again (`accept-owner --keypair`, the local stand-in for `--ledger`). Refused as expected: `list --owner` with the fund
wallet's or a foreign key; `propose-owner` before the ceremony was complete or by a foreign key; `accept-owner` before the proposal or
by another key (the tool, and rook_core on chain: custom error 0x1773 `NotOwner`); a tampered transaction on the page (other program,
registry, instruction, signer, already signed) and a wallet that alters the message; `verify-owner` before the acceptance (exit 1).
Without `--skip-new-upgrade-authority-signer-check`, `solana program set-upgrade-authority` failed with "missing signature for
supplied pubkey". No Ledger was attached: `--ledger` was exercised only up to "no hardware wallet found".

**The listing ceremony** (`rook-ceremony list`, audit A6-02; step 4 above), signed and paid by the registry owner during the setup,
the ceremony payer (`--owner ~/rook-keys/ceremony-payer.json`).

`custody.json` holds the canister's keys from its deployment record: `custodian` (its Ed25519 Solana key), `poster` (EthNft floor
poster; default the custodian), `eth_custody` (its threshold-ECDSA address), `btc_custody` (its `bc1p…` taproot address), optional
`per_slug` overrides and `per_slug.<slug>.wx_uri` (the wX image JSON; empty is allowed and set later with the owner-only
`set_wx_metadata`). The tool runs preflight, initializes treasury and desk, then for each `i` in `listing_order`: asserts
`Registry.listing_count == i`, `list_collection` (name and kind from `listing_order`, custody fields from `custody.json`) and reads
every field back; `init_wrapper` (wX metadata name / symbol from `nft-pilot.json`: wMILADY … wBFROG … wNATCAT), `init_module` (the launch
file's `init.init_module.params`), `add_collection`; `register_partner` (partner read back: listing, registry, module, vault,
wrapper, wX mint, claimer = `claimer(listing(i))`); `register_partner_config` with **the config `day1-plan.json` names for that
slug**. A listing that exists with any other field, a `listing_count` out of step, or a config that does not pay that listing's
claimer stops everything. The fund wallet never signs here: ownership moves to it afterwards (steps 5–7). Then anyone (the canister)
cranks `open_vault` and `back` for every coin that graduated meanwhile.

## 5. The fund

- **Direct contributions:** plain SOL transfers to `GGgXnNgtY4ySfz8PLAAwkoh8Ztj7p6jrM9towSaa6Wuy`.
- **ROOK:** its creator fees accrue in DBC. The fund wallet claims them with DBC `claim_creator_trading_fee`, for example from
  Meteora's creator dashboard, signed by the wallet.
  - **Same family** as the collections: 1B supply, 6 decimals, 85 SOL threshold, the same anti-sniper schedule, LP permanently locked 50/50.
  - **Curve:** with a 0 % migration fee, the fixed 1B supply validates only with a curve designed for 0 % (Q1 §6.1). So `design_curve.mjs 1000000000 20 85 0 6` is used. Start mcap ≈ 26.6 SOL; the whole 85 SOL goes into the DAMM v2 pool at graduation (mcap ≈ 425 SOL).
  - **What ROOK can raise:** curve fees are ≈ 1 % after the first 10 minutes (50 % at launch, decaying). At 80 % to the creator, **≈ 1,234 SOL of ROOK curve volume pays the 9.87 SOL deploy target** (≈ 1,350 SOL with the ≈ 0.93 SOL listing ceremony of 17 listings) on its own, less with the anti-sniper minutes. After graduation, both locked LP positions (partner and creator, both the fund wallet) earn their share of the 1 % pool fee.
- **Status line for the site:**

  ```sh
  node ops/solana/fund-status.mjs [--plan ~/rook-keys/day1/day1-plan.json] [--json]
  ```

  On 2026-10-09 it printed: "0.00089088 of 9.54068916 SOL raised for Rook's deploy deposit" (the target is now ≈ 9.87 SOL with the
  2026-10-10 build, plus ≈ 0.93 SOL for the listing ceremony of 17 listings). It is read-only, and `--json` has a `line` field for the site and a
  `launch_line` that includes the listing ceremony.

### Public explainer (for the site)

> **Funding Rook's on-chain deposit.** Solana charges a storage deposit (≈ 9.9 SOL at peak, ≈ 7.2 SOL held for as long as the programs
> exist, plus ≈ 0.9 SOL to list the 17 collections) to put Rook's two programs on-chain. Rook is raising it in public. Anyone can send SOL to the fund wallet
> `GGgXnNgtY4ySfz8PLAAwkoh8Ztj7p6jrM9towSaa6Wuy`, and 80 % of the trading fees of the ROOK coin go to the same wallet. Its balance
> is public, and so is the progress line on this page.
>
> **A contribution is a donation to the deploy deposit.** It buys nothing: no token, no share, no claim on fees, no refund, and no
> promise of any return. ROOK is a memecoin with no backing and no rights attached. Buy it only for its own sake.
>
> Until the programs are deployed, Rook's collection coins launch and trade normally. The backing share of every graduated coin waits
> inside Meteora's program and is converted to NFT backing once the programs are live.
>
> The wallet is a single key held by Rook's operator. It also becomes the programs' upgrade authority, so that one key is a single
> point of failure until control moves to a multisig, and the programs' code at deploy time is whatever that key holder deploys.
>
> The long-term deposit is not refunded to anyone. It can be recovered only by **closing** the programs, which ends them for good:
> every pending and future backing claim becomes unclaimable, and the deposit goes to the upgrade authority, not to contributors.
>
> Until the deploy, the backing of every collection coin depends on two things: Rook's operator keeping the two program keys safe
> (whoever holds them can deploy any code at those addresses, and if they are lost nobody can), and the fund reaching its target. If
> the deploy never happens, the backing share of every graduated coin (42.5 SOL each) stays inside Meteora's program forever.

## 6. Running costs: the canister and RPC (tasks 4–5 of 2026-10-09)

**IC ticks.**
- **Cadence:** the canister has no timer. The runner calls `tick()` every 600 s (120 s while a home transaction is out, 900 s when everything is dormant).
- **At rest:** a quiet tick reads ≈ 2.5 B cycles of Solana outcalls (genesis, three clocks, `getMultipleAccounts` per flow; no line traffic). It costs ≈ 5.5 B with the Ethereum and Bitcoin reads. Ticks that sign cost 17–20 B (26 B per threshold signature).
- **Changed today** (`autobuyer/canister-rs`, 940 tests green):
  - The rook_backing vault listing (`getProgramAccounts`, every 6 h per collection) now reads 64 entries first, at a 16 kB cap. It used to reserve 0.9 MB per provider.
  - Ticker reads are capped at 5 kB instead of 16 kB.
- **Cost, 1 T cycles = 1 XDR = $1.354:**

  | | Before | After |
  |---|---|---|
  | No active collection | 1.45–1.71 T/day = **$1.96–2.32/day** | 0.55–0.81 T/day = **$0.74–1.10/day** |
  | 3 active collections | ≈ 5.6 T/day = **$7.6/day** | ≈ 4.6 T/day = **$6.2/day** |

- **Proposed, not done:**
  - share one clock read across the three flows (−0.08 T/day; needs a deadline review);
  - merge each flow's account reads (−0.06–0.10 T/day);
  - a 1,800–3,600 s runner cadence when nothing is in flight (÷3–6 at rest; wake requests and redemptions wait longer);
  - lower the Ethereum `eth_call` cap from 16 kB to ≈ 4 kB;
  - measure, then lower, `SIM_BYTES` (24 kB).

  Fewer providers is rejected for any read that drives a signature.

**Requests per provider per month.** Every replicated outcall reaches each provider from all 13 replicas.
- At rest: (14 + 3A) × 13 per tick (A = active collections), about **0.79 M/month with A = 0** and 1.29 M with A = 3.
- Busy (A = 3): ≈ 2.5 M/month.
- Ceiling (always pending at 120 s): ≈ 6.5 M/month.

**RPC recommendation: Helius (Developer, $49/month, 10 M credits) + Ankr Premium pay-as-you-go (≈ $50 per M requests), called
directly. ≈ $90–175/month at 0.8–2.5 M requests per provider.**
- **Why these two:** they are the only two keyed providers whose RPC hosts have IPv6 (AAAA) records. Triton (`rpcpool.com`), QuickNode, Alchemy, dRPC and Chainstack are IPv4-only, so IC replicas would reach them through the shared IPv4 proxies that rate-limited OnFinality in devnet run 3.
- **Keys:** the URL keys are visible to every node operator of the subnet. Use dedicated keys for the canister only, with a credit cap and alerts, and rotate them through the provider-config path. An IP allowlist of 13 replicas is brittle, since membership changes with NNS proposals.
- **Fallback:** Helius + PublicNode (`solana-rpc.publicnode.com`, keyless, IPv6, free, no SLA).
- **DFINITY's SOL RPC canister** (`tghme-zyaaa-aaaar-qarca-cai`, controlled by the NNS root): **not the primary route.**
  - Typed methods: `getAccountInfo`, `getBalance`, `getBlock`, `getSignatureStatuses`, `getSlot`, `getTransaction`, `sendTransaction`, and others.
  - Not typed: `getMultipleAccounts`, `getProgramAccounts`, `simulateTransaction`. Those go only through a generic `jsonRequest` with no slot-stripping transform, so providers rarely agree.
  - Consensus is Equality or Threshold, so technically it meets "≥ 2 agree". But the agreement happens inside a canister Rook does not control, with provider keys it does not hold.
  - It costs ≈ 1.7 B cycles a read, ≈ 12 times our own outcall.
  - Use it at most as an occasional cross-check from the monitor.
- Confirm Ankr's per-call rate and Helius's credits for `simulateTransaction` before buying.

## 7. [Appended 2026-10-10 by the canister work, decision D-i] Deploy order: the canister is the registry keeper

*(A separate section so the ceremony edits above stay theirs; docs/v9/solana-migration.md §2.9 D-i is the decision.)*

The registry keeper is the canister's Solana address (no keeper service). That address is derived from the canister's threshold
Ed25519 key, so it exists only once the canister does. rook_core needs no change: `propose_roles(keeper)` applies AT ONCE for the first
keeper (owner-signed; only a later change waits 7 days). The order:

1. Day 1 (§3): unchanged (no canister, no programs).
2. Create and install the canister with `home = Solana { …, rook_backing: <reserved id>, drand_urls: null }`; read `sol_address()`
   (the custodian, the EthNft poster and now the keeper) and the Ethereum / taproot custody addresses: they fill `custody.json`.
3. `deploy-later.sh` (§4): deploy, `init_registry`, `init_backing`, handover. Unchanged.
4. `rook-ceremony list` (the ceremony payer signs, as the registry owner during the setup; §4): **first** `propose_roles(keeper = custody.json.custodian)` when `Registry.keeper` is
   unset, read back (`keeper == custodian`, not paused); refuse if the registry already names another keeper (moving it is then a
   7-day `propose_roles` / `accept_roles`). Never name a keeper-service key: the D-a keypair of the rehearsal is not used on mainnet.
5. Fund the canister's Solana address (≈ 0.5 SOL a month at launch volumes: crank fees, vault rent 0.0047 SOL a coin, 0.0012 SOL per
   proven drand round, 0.002 SOL per coin's first fee sweep).
6. From then on the runner's `tick()` drives everything: the canister opens vaults, backs graduations, sweeps fees past 0.05 SOL,
   applies treasury changes, allocates and distributes, proves drand rounds (verified BLS) and resolves redemptions, and posts the frog
   floor from the runner's readings. Only claiming ROOK's creator/partner fees into the fund wallet is manual.

## 8. [Appended 2026-10-10 by the fueler work] Fueler: the ops share pays the canister's cycles

*(A separate section; the reference is `docs/v9/fueler-solana.md`.)*

The fueler (`ops/src/fueler.ts`, `FUEL_HOME=solana`) holds a Solana hot key that is rook_core's `Treasury.ops`: every `allocate` pays it
10 % of the treasury's income, and it converts SOL to the canister's cycles (Relay SOL → ETH on Ethereum → ckETH → ICPSwap → ICP → CMC),
within caps, forwarding the surplus to the fund wallet. Nothing on day 1 changes. After `deploy-later.sh`:

1. Generate the hot key on a clean machine (`solana-keygen new --no-bip39-passphrase -o fueler-hot.json`); seal its JSON in the fueler's
   Railway service as `FUELER_SOL_KEYPAIR`. Fund it with **about 0.05 SOL** (fees).
2. Name it at the listing ceremony (the registry owner, the ceremony payer during the setup, signs `set_ops` right after
   `init_treasury`, read back):

```sh
ops/rehearsal/sol-devnet/tool/target/release/rook-ceremony list --launch ~/rook-launch/launch.json \
  --plan ~/rook-keys/day1/day1-plan.json --custody <custody.json> --rpc <keyed mainnet RPC> --rpc2 <second provider> \
  --canister-sol <the canister's sol_address()> --ops <the fueler's hot key address>                      # dry run
#  … then add: --owner ~/rook-keys/ceremony-payer.json --send       (Type 'list'; then propose-owner / accept-owner / verify-owner, §4)
```

   (`--canister-sol` is the keeper step of §7; it must equal `custody.json`'s custodian.) Set later instead, the first `set_ops` still
   applies at once; replacing an operations address waits a day and anyone applies it with `apply_ops`.
3. Deploy the fueler's Railway service (`ops/Dockerfile.fueler`, one replica, a `/data` volume; variables from
   `deployments/launch/railway/fueler.env.example`: `FUELER_SOL_PROGRAM` = rook_core's reserved id, `FUELER_RPC_SOL`, `FUELER_EVM_KEY`,
   `FUELER_IC_IDENTITY_PEM`, `RELAY_API_KEY`, `FUELER_BUYER`, `FUELER_TIMELOCK`). `FUELER_MODE=dry` for a day, then `send`.
4. Add the monitor's `fueler` block with `sol_ops` (`docs/v9/fueler-solana.md` §6) and the fueler's `/healthz` to `liveness`.

`ops/solana/fund-status.mjs` does not count the fueler's 0.05 SOL: it is an operating cost, not part of the deploy deposit.

