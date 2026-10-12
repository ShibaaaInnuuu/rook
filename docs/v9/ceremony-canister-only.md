# The launch ceremony, canister-only (W10)

Design: `docs/v9/canister-only-solana.md` §12 (sequence, SOL, cycles), §16 (2026-10-12: every working collection pinned, each opened
when a creator first asks for it). There is no
program to deploy and no `rook-ceremony` run: the canister makes every wX mint, every Metaplex metadata and every DBC config itself, in
the order creators ask for them (`coins_request_open`), as `K_main`'s float covers each (`coins::open`, ≈ 0.023 SOL a collection). The operator's work is the install, the
pins, the float, and ROOK.

## 0. Before

- The build: `autobuyer/scripts/reproducible-build.sh` (the wasm is ≈ 1.88 MiB gzipped since symbols are stripped, under the 2 MiB single-install limit; the chunked path still works,
  `autobuyer/scripts/chunked-install.sh`, or the timelock's chunked proposal).
- Cycles (decision 2026-10-12): start with **≈ 20 T (≈ $27)** on the operator's cycles ledger: the fiduciary creation fee (≈ 1.3 T)
  and a few quiet days (≈ 2 T a day on 34 nodes; a launch-week day is ≈ 12 T), and above all the canister's **8 T** floor for its own
  Ethereum sale scan: under it no Ethereum collection gets a FIRST floor (found in the intent rehearsal). Keep the canister over
  8 T until each Ethereum collection has posted once. Not the 30-day reserve of §12.3: the fueler runs from the first fees, and the
  monitor's low-cycles alert is the cue to top up by hand until it has two clean runs.
- Decisions (§14, 2026-10-12):
  - **Subnet (O-10):** fiduciary, `pzp6e-ekpqk-3c5x7-2h6so-njoeq-mt45d-h3h6c-q3mxf-vpeq5-fk5o7-yae`. Re-read it on the IC dashboard
    first. Create the canister and the timelock there (`--subnet <id>`).
  - **RPC (O-13):** `rpc_urls = ["https://solana-rpc.publicnode.com", "https://mainnet.helius-rpc.com/?api-key=<a key for the canister only>"]`.
    Helius Free unless the fiduciary devnet rehearsal shows 429s or the month runs short; then Developer.
  - **Guardians (O-7):** no outside guardians. The operator makes five distinct keys, each on its own device: proposers p1, p2;
    guardians g1, g2, g3 (veto k = 2). Keep g3 offline as the backup. The canister's pause guardian is g1. Read the five principals
    back before the install: the timelock refuses a principal named twice or in two roles.
    Made 2026-10-12 on the operator's Mac (dfx identities in the macOS keychain): p1 `jwtyr-bjz34-xpk3j-guwcn-igu4r-cdpw5-cybm6-xx5lt-nvrcm-qvlka-fqe`,
    p2 `jd5fp-gbdia-l5jyl-l64ph-nuwbd-sltlv-ruwnv-gxpyf-7yofw-ba5zw-sqe`, g1 `4423w-dirhd-6mmzf-zrn3b-ukl6j-3p4mb-xhljt-gu67m-zojhm-yqdwk-fae`,
    g2 `ght7l-5bmui-5xas3-jjwzi-xm4ac-g57wd-hby3u-su46d-mefmb-ehiua-mae`, g3 `5re2m-u7frt-k642z-nzj7d-kskmn-67zi5-ewpsv-ghh7h-3bqkl-f5tz6-oqe`.
    g2 and g3 were moved off the Mac on 2026-10-12 (offline copies held by the operator); the Mac holds p1, p2, g1.

- Railway (2026-10-12): `runner`, `indexer-sol` and `fueler` exist, empty (no source until the ceremony), their keys stored:
  runner `SOL_RPC_URLS` (Helius key 2 + PublicNode); indexer-sol `SOLANA_RPC_URL`/`HELIUS_API_KEY` (Helius key 3); fueler `FUEL_HOME=books`,
  `FUELER_MODE=dry`, and its fresh keys (files in `~/rook-keys/fueler/`, mode 0600): Solana hot key
  `D1EDmAdkmCuPx2QV3k4LHZrPbUsBmvEr69KW1LDUbuuT` (the canister's `treasury_set_ops` argument), Ethereum address
  `0x9fccc7Cee4f24d2ffD9cFAE9cbb4818Dcf3e26D0`, IC identity `rook-fueler-2` = `aedxx-hifho-tdyvi-dk67k-ab34b-hxnpu-cjwpf-el6tb-npg7l-famwk-iqe`.
  Still to add: `RELAY_API_KEY` (fueler), the runner's `OPERATOR_IDENTITY_PEM`, `OPENSEA_API_KEY`, `UNISAT_API_KEY`, and each
  service's canister id.

## 1. Install (controller: the timelock)

The argument is made by `ops/ceremony/mainnet-init.mjs` (from `deployments/launch/ceremony-input.json`), the timelock's by
`ops/ceremony/timelock-init.mjs`; the ordered commands, cycles budget included, are `docs/v9/ceremony-commands.md`.

`Init.home = Solana { cluster: "mainnet-beta", rpc_urls: [two], rook_core: "books", nonce_seed, blockhash_fallback: null,
rook_backing: null, drand_urls: null }`. `rook_core = "books"` is the only value the install accepts (decision D-l, `canister::validate_home`:
any program id is refused, and so is a `rook_backing`): the canister keeps the books itself; the program readers, builders and keeper
(`solread`, `solix`, `solcrank`) are gone from the code; the coins, the pool desk and the treasury cranks run.

**Custody caps.** Leave `custody_caps` empty (`null`): the defaults then name every listing at stage 0, Solana NFT collections included
(`sol:<collection>`). An install that writes its own cap list must name each Solana NFT collection too, or that collection's buys
are refused as `NotListed` (audit C1-02's fix checks the policy as written).

For the Ethereum collections `eth_nft.collections[i].wrapper = module = "books"` (`Col.books`). For a Robinhood Chain collection
(`robinhood-everything.md` §3, "Built"):
- `evm_chains` gets an entry for chain 4663:
  - two 4663 RPCs;
  - `seaport` 1.6;
  - `seaport_codehash`: the keccak of Seaport 1.6's code **read back on 4663** at the ceremony
    (`cast keccak $(cast code 0x0000000000000068F116a894984e2DB1123eB395 --rpc-url <4663>)`), never Ethereum's;
  - its fee caps.
- The collection names `chain_id = opt 4663` (and StonkBrokers `token_bound = opt true`). The frog collection's `Init.wrapper` is
unused on the books home (the Inscriptions listing in the books is the frog collection).

Then read the canister's addresses: `sol_address` (`K_main`), `sol_address_of("claimer")` (`K_fc`), `sol_address_of("treasury")`
(`K_tre`), `evm_address` (the Ethereum custody), `custody_address` (the taproot custody), `sol_address_of("coll:<i>")` for i in 0..38.
`sol_address` is also the Solana collections' custodian: the canister's own wallet holds their NFTs.

## 2. The pins

```
cd autobuyer/canister-rs
cargo run --example launch_pins -- --evm-custody <evm_address> --btc-custody <custody_address> --sol-custodian <sol_address> \
    --uri-base https://userook.fun/wx/ --out ../../deployments/launch/canister-only
```

**No launch set, no picking (operator decisions 2026-10-12):** the ceremony pins every collection with a pipeline, the first 38 of the
canister's registry (`--count 38` is the default, and the most `--count` takes): the 11 Ethereum collections (`EthNft`,
`--evm-custody`), Bitcoin Frogs and the 14 other Ordinals (`Inscriptions`, each with its own `membersRoot`, `--btc-custody`) and the
12 Solana collections (`SolNft`, `--sol-custodian <sol_address>`: the canister's own wallet holds them). The example refuses before
writing anything if a pin's custody argument is missing. `--include-ordinals` is still accepted and does nothing (the Ordinals are in
the default; the example prints a note). The default writes 38 pins and prints "38 pins written … 0.858 SOL were all 38 to open".

The three **Robinhood Chain collections** (`robinhood-collections.json`: StonkBrokers, Chain Mancers, Rekt Tradooors, `ORDER`
indices 38–40) are **not** in the default (audit C4-03). They are pinned after the ceremony, once the 4663 `evm_chains` entry is read
back, with `--include-robinhood`. The canister also refuses to open an `EthNft` pin on a chain other than Ethereum until that chain is in
its `evm_chains` (`coins::open::pipeline_ready`): a request for one answers "cannot open yet". To pin them:
1. Give the canister the collections in `eth_nft.collections` (`chain_id = opt 4663`, `token_bound = opt true` for StonkBrokers,
   `wrapper = module = "books"`) and the 4663 `evm_chains` entry (§1), with Seaport 1.6's code hash read back on 4663. At the ceremony
   this is the install argument. After it, an upgrade carries it, under the timelock.
2. Run `launch_pins … --include-robinhood` into a fresh `--out` and `coins_pin` only `38-stonkbrokers.args`, `39-chainmancers.args`,
   `40-rekttradooors.args` (the earlier indices are taken; a second pin of one is refused). `pins.json` labels them `"robinhood"`.
3. Put the OpenSea slugs in the runner's `ETH_NFT_SLUGS`, and set `OPENSEA_CHAIN_4663` if OpenSea's name is not `robinhood`. Set the
   reconciler's `RECONCILE_RH_RPC_URLS`.

A later 4663 collection: run `! OPENSEA_API_KEY=… node scripts/measure-robinhood-nfts.mjs`; a collection that passes prints a
`graduation-tiers.json` entry and a `robinhood-collections.json` entry (the generated `symbol` is `w` + at most 9 characters: the
canister's 10-byte limit). It takes the entry's `index`, else 41 upward in file order, and is pinned as above.

One `coins_pin` argument per collection in `deployments/launch/canister-only/<index>-<slug>.args`, and `pins.json` to review: the
floor and tier of `graduation-tiers.json` (the Solana collections: `solana-collections.json`'s floor and `threshold_wx`),
`T_wX = max(3, ⌈tier / floor⌉)`, the curve (Meteora's `designCurve` in wX), the value-based parameters (§4.3). Each pin is checked
as the canister checks it (`coins::open::register`). The registry's head (2026-10-10 floors): BAYC `T_wX` 4, Milady 5, Mad Lads 11,
SMB Gen2 7, Bitcoin Frogs 29, Pudgy Penguins 5, Azuki 7. The `<slug>.json` metadata files the pins point to are in the site,
`web-rook/public/wx/<slug>.json` (all 41: the 38 and the 3 of `--include-robinhood`), served at `https://userook.fun/wx/<slug>.json`
(the default uri base) with each picture at `wx/img/<slug>.png`. They are generated, not hand-written: `cd web-rook && node --import
tsx scripts/wx-metadata.mjs` writes each one's name and symbol from `unlock.rs`'s `ORDER` (`robinhood-collections.json`'s for the
Robinhood Chain three, as `launch_pins` does), a one-sentence description, the collection's page as `external_url`, and a 512 × 512
PNG of the rook mark over the wX symbol (no collection art); `web-rook/test/wx.test.ts` fails if one is missing, stale or differs
from what the pin puts on chain. Deploy the site before the pins (or with `--uri-base` pointed elsewhere) and check one:
`curl -s https://userook.fun/wx/bayc.json`.

`dfx canister call <id> coins_pin --argument-file deployments/launch/canister-only/00-bayc.args` … for the 38. A pin is refused
if malformed or its index is taken; `coins_collections` shows them as `Pinned`. **Nothing opens yet:** the canister opens a collection
when someone first launches a coin on it (the site's "Open it" calls `coins_request_open(index)`; `coins_collections` then shows
`Requested`, `Opening`, `Open`). `coins_set_auto_open(true)` (operator; default off; refused while paused) opens the pinned ones in
index order instead.

**The queue's bounds (audit C4-02, C4-05):** at most 8 requests pending (a day of openings); at most 4 accepted an hour from everyone,
2 of them from the anonymous principal (the site's visitors); a named principal holds one pending request at a time and 2 an hour.
`inspect_message` refuses the ingress of any request that would change nothing (not pinned, open, refused, not ready, already
requested, over a bound), and every request is refused while the module is off (`coins_set_off`) or with no Solana home.
`coins_open_queue` lists the queue with each request's position and what the pass waits on for it; a request the pass waits on for a
reason of its own (a pinned mint the chain lacks, a step refused by simulation) is passed over after 30 minutes for the next one, and
keeps its place (C4-04).

**A refused collection (audit C4-01):** a mint or config address an outsider pre-funded is not a refusal any more (the canister makes
the account over the lamports). A collection refused for another reason (a wrong account at its address, a config DBC refused three
times) stays refused until the operator has looked: `coins_retry(index)` (operator) pins it again under its own index and symbol and
queues it; the pass reads the chain afresh and refuses again what is still wrong.

## 3. The float

Send **≈ 0.2 SOL to `K_main`** (§16: the 0.05 SOL the money flows keep, the nonce accounts 4 × 0.00145, ATAs and fees, and about
eight openings at ≈ 0.023 SOL; the rest are paid from fees). The canister opens the REQUESTED collections, oldest request first, one
step a pass on the crank line: the wX mint + metadata (one transaction), then the DBC config (one transaction), then it reads both back
(`check_wx_mint`, `check_config`) and lists the collection in the books. `coins_collections` → `Open`; `coins_log` shows what waits
and why, and the queue. Nothing opens while the float is under a collection's cost, and nothing opens unasked (auto-open off).

## 4. ROOK (open)

ROOK is SOL-paired with `fee_claimer = K_fc` (§16). The coins module opens wX-quoted collection coins only; ROOK's config and its
first buy are the operator's (Meteora's own launch tooling, ≈ 0.027 SOL), with `fee_claimer` and `leftover_receiver` set to `K_fc`.
ROOK's fees arrive at `K_fc` as SOL: the cranks sweep them to `K_tre` (`TreasuryStep::ClaimerSweep`, once 0.05 SOL sits above the
key's rent) and book them as income when the transfer lands.

## 5. Floors, mint, pools

- **No wake (default-live, operator decision 2026-10-12; audit C2 §4).** A fresh install starts **every** Inscriptions listing live,
  Bitcoin Frogs (4) included: no `frog_wake(4)` (nor any other) is called. `frog_collections` shows each `live = true`. Before a
  listing's first floor is posted the canister sets, where nothing is set, its floor sink (its module), its purchase limit and minimum
  and its Ordinals Wallet creators from the launch files (`frogsol::launch`); a controller's own `set_*` calls still win. No listing
  mints or buys until a floor reading is accepted. `frog_sleep(coll)` is the operator's brake (refused while the collection has work
  open, holds the Relay leg or has an unread outcome on the line); `frog_wake(coll)` lifts it.
- The runner hands in floor readings (`eth_floor_reading`, and `frog_floor_reading(coll, …)` for every live Inscriptions collection:
  `frog_collections` lists them, one put to sleep is refused). The canister posts them (books). A listing's FIRST floor is refused
  unless it is within ±50 % of its launch floor (`graduation-tiers.json` `floor_sol`, audit C2-12): a refusal is a loud tick line and
  nothing is posted; re-measure before the ceremony if a floor has moved that far.
  A collection's mint opens when its floor is fresh. A Robinhood Chain collection's floor comes from OpenSea on 4663
  (`OPENSEA_CHAIN_4663`), and the runner reads its side through `eth_nft_state_of(4663)`.
- Intents: `open_intent` (anyone), `intent_status`, `quote_mint`, `quote_burn`.
- Pools: none at launch; each collection's pool desk opens its DLMM pair once its fee income reaches the opening float (§5.3,
  `pool_desks`).
- Treasury: `treasury_set_ops(<the fueler's hot key>)` once; `treasury_status`.

## 6. Site and monitor

The site switches to the live canister id (W12: the two-hop buy, the intent desk, the Reserves page); the monitor to the new views
(W11: `treasury_status`, `pool_desks`, `coins_log`, `desk_status`).
