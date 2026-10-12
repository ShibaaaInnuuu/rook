# The Solana devnet rehearsal — runbook

Status: **kit ready 2026-10-09**, branch `golive-2026-10-08`. The local parts were run the same day (§6); the devnet + IC part is the
operator's to run (§3). Plan: `solana-migration.md` §7 P4; programs: `rook-core-as-built.md`, `rook-backing-as-built.md`; canister:
phases A/B1/B2 (`autobuyer/canister-rs/src/sol*.rs`, `frogsol.rs`); signing spike: `spikes/q3-ic-ed25519-durable-nonce.md`
(`ops/rehearsal/sol-sign/`, whose IC pattern and cleanup fix this kit follows).

Kit: `ops/rehearsal/sol-devnet/`

| File | What |
|---|---|
| `run.sh` | the orchestrator: `--local-validator`, `--local-replica`, `--devnet` (header = usage) |
| `tool/` | `rook-devnet`, the operator's side: rook_core / rook_backing / Meteora DBC instructions built from the programs' own Rust types, signed with operator keypairs, sent over JSON-RPC; every step simulated first, recorded with its signature and CU in `<job>/state.json` |
| `drive.py` | the canister's side through `dfx`: `addresses`, `checklist` (devnet + IC), `smoke` (local replica) |
| `canister-init.mjs` | the throwaway buyer's install argument (`home = Solana(devnet)`), encoded with the monitor's own IDL and decoded back; `--books` (W15): `rook_core = "books"`, the launch set's Ethereum collections at `wrapper = module = "books"` |
| `monitor-check.mjs` | the monitor's own read of `sol_line` (`monitor/src/ic.mjs`) and its alerts (`monitor/src/ethnft.mjs`); `--books`: the books home's views and `monitor/src/books.mjs`'s alerts |
| `books.py` | W15 (§8): the canister-only rehearsal's phases (`addresses`, `pins`, `fund`, `open`, `checklist`) and the offline `parse` of the saved views |
| `views-decode.mjs` | a saved reply (`tests/fixtures/views/*.hex`) decoded as the monitor decodes it, for `books.py parse` |
| `test/` | `node --test ops/rehearsal/sol-devnet/test/*.test.mjs`: the books argument against the did, the dry run, the parsers (offline) |
| `ops/solana/launch-params.mjs` | the launch file: every ETH-denominated value converted at the live SOL/ETH rate (decision D-c) |

## 1. What it proves, what is stubbed

| | Real on devnet | Stubbed, and why |
|---|---|---|
| Programs | the SBPF v3 binaries `anchor build` makes, deployed with the operator's key (upgrade authority = registry owner) | — |
| Registry | `init_registry(owner, Signet)`, guardian, keeper, treasury, desk with the launch file's lamports | the **keeper service** is an operator keypair (decision D-a: the keeper is a separate service, not built yet) |
| Listings | EthNft (Milady, chain 1) and Inscriptions (`tb1p…` custody = the canister's), each with wrapper + module at the launch file's params; a third Inscriptions listing for the program drill | the EthNft listing's **poster is the keeper key**, not the canister: the canister's own ETH floor post needs OpenSea evidence (`eth_floor_evidence`), which the runner gathers with an API key; the canister-as-poster path stays covered by `tests/ethnft_sol.rs` |
| Canister line | threshold Ed25519 (`test_key_1`) on IC mainnet, durable-nonce line opened by the canister, every step simulated before it is signed, landed and read back at `finalized` through two devnet providers; the genesis check | — |
| ETH buy | the canister's **advance** (lamports module → canister) and its **return_advance** | **Relay has no devnet**: no SOL → ETH order; the operator's exit `eth_give_back` returns the advance (as after a refused quote). No Ethereum transaction is ever sent; Ethereum mainnet is only read |
| Frog buy | the canister's **advance** and **return_advance**; the keeper's floor post | **Relay has no devnet**: no SOL → BTC order; `relay_give_back` returns it. A real signet purchase (`--frog-offer`, an operator-made seller PSBT of a signet inscription in the listing's tree, the float funded with signet sats: `frog_funding.float_first`) leads to `mint_item` with a proof and `item_arrived`; without it the canister mints nothing |
| Tickets | redemption to a `tb1p…` address, keeper-proven drand round, `resolve`, `note_sent`, `record_delivery`, `mark_undeliverable` — **by the program drill** (operator key as custodian) on devnet's binary | by the **canister** only with a frog it minted (`--frog-offer`); drand is keeper-attested until Q4 (`prove_round`) |
| DBC | (`--dbc`) Meteora DBC, DAMM v2 and the "Customizable" DAMM config **exist on devnet at the mainnet ids** (checked 2026-10-09: DBC deployed at slot 508,712,305, DAMM v2 at 503,166,267): config with the claimer PDA as fee claimer, launch, `open_vault`, buy to a 2 SOL threshold, permissionless migration to DAMM v2 | `back` → rook_core `convert` → `redeem_backing` **in the same run**: rook_backing's `["converter"]` PDA is allowed at `init_registry` (no 7-day wait; rook_core as-built §8.8); a 2 SOL threshold (dynamic supply) instead of production's 85 SOL |
| Upgrade | `install --mode upgrade --upgrade-unchanged` with the line out, the line identical after | — |
| Monitor | the monitor's code reading the IC canister anonymously, its `sol-line` alerts | the monitor service itself is not deployed for the run |

## 2. Prerequisites

- **Tools**: `solana` 4.1.2 (Agave, with `solana-test-validator`), `anchor` 1.2.1, Rust 1.89 (`solana/rust-toolchain.toml`), the
  canister's toolchain (`autobuyer/canister-rs`, `wasm32-unknown-unknown`), `dfx` 0.32, `node` ≥ 22 (`monitor/` installed:
  `cd monitor && npm ci`), `python3`, `curl`.
  `export PATH="$HOME/.local/share/solana/install/active_release/bin:$HOME/.cargo/bin:$PATH"`.
- **The devnet payer** — the operator's own keypair file, **outside the repository** (`run.sh` refuses a path inside it). It becomes
  the upgrade authority of both programs **and the registry's owner on devnet for good** (the program ids are the repository's dev
  keys in `solana/target/deploy/*-keypair.json`, git-ignored; `init_registry` runs once per program id): keep it for every later run.
  Make one with `solana-keygen new -o ~/rook-devnet-payer.json` and fund it at <https://faucet.solana.com> (devnet; 5 SOL a request,
  rate-limited: sign in with GitHub for more) or `solana airdrop 2 -u devnet -k ~/rook-devnet-payer.json`.
- **Devnet SOL** (2026-10-09 sizes):

  | Use | SOL | Comes back? |
  |---|---|---|
  | rook_core program data (867,112 bytes) | 6.04 | locked while deployed (`solana program close` returns it, see §7) |
  | rook_backing program data (510,712 bytes) | 3.56 | same |
  | deploy buffers (one program at a time) | up to 6.04 more, transient | yes, at the end of each deploy |
  | registry, treasury, desk, 3 listings × (listing, wrapper, wX mint, item set, module, vault, module wX) | ≈ 0.2 | no |
  | role keys: keeper 0.2, guardian 0.05, custodian 0.5 | 0.75 | left on the keys |
  | holder: ETH listing floor × 1.9 (8 SOL floor: 15.2), frog floor × 1.9 (2.85), +4 for the drill | ≈ 22 | mostly: the checklist burns the units back at the floor |
  | the canister's address (fees, nonce rent 0.0015) | 0.05 | no (stays at a deleted canister's address) |
  | `--dbc`: buy to the 2 SOL threshold + rents | ≈ 3.5 | no |

  **≈ 36–42 SOL at the peak** (≈ 21 with `--skip-eth`, which leaves the ETH steps out). The ETH floor is 8 SOL because the canister
  only advances when the advance reaches Ethereum covering the floor + 1 % + the 0.003 ETH gas reserve after Relay's fee bound and
  the FX margin (`solhome::advance_covers`): below ≈ 7.2 SOL a unit no ETH buy starts (`--eth-floor-sol`).
- **Cycles** — a dfx identity whose **cycles-ledger** balance holds ≥ 3.5 T (`dfx cycles balance --network ic`): the creation fee
  (500 B on a 13-node subnet, ≈ 1.3 T on 34 nodes) plus the ticks (≈ 1.1 B each at local-replica prices; ≈ 40–80 ticks) and the
  threshold signatures (≈ 26 B each, ≈ 6–10 of them). `--cycles` (default 3 T) is what the canister is created with; the rest is
  withdrawn back at the delete.
- **A subnet**: `--next-to <the buyer's canister id>` (the launch subnet), `--subnet <id>`, or `--subnet-type <type>`, as sol-sign.
- Optional: `--frog-ids <file>` (real signet inscription ids, one per line, added to the frog tree) and `--frog-offer <json>`
  (`{ "frog": <want id>, "inscription": "<id>", "psbt_hex": "…", "to_sign": [..], "proof": ["<hex>", …] }`; the proof from
  `rook-devnet proof --listing frog --inscription <id>`) for the real signet purchase.

## 3. Commands

```sh
# 0. the launch file (also written by run.sh when --launch is not given): >= 2 public sources, refused over 1 % apart
node ops/solana/launch-params.mjs --out ~/rook-devnet/launch.json --cluster devnet --bitcoin-network signet
node ops/solana/launch-params.mjs --check ~/rook-devnet/launch.json          # the drift since it was written

# 1. free dry runs (no IC, nothing written on devnet)
ops/rehearsal/sol-devnet/run.sh --local-validator --dbc       # SBPF v3 deploy + init + refusals + drill + DBC (cloned from devnet)
(cd ops/rehearsal/sol-devnet && dfx start --clean --background)
ops/rehearsal/sol-devnet/run.sh --local-replica              # the buyer on PocketIC (key_1): install arg, tick against devnet, upgrade
(cd ops/rehearsal/sol-devnet && dfx stop)

# 2. the rehearsal (devnet + a throwaway canister on IC mainnet with test_key_1)
ops/rehearsal/sol-devnet/run.sh --devnet --payer ~/rook-devnet-payer.json --next-to <buyer canister id> \
  --job ~/rook-devnet/job-1 --launch ~/rook-devnet/launch.json [--dbc] [--frog-ids f.txt --frog-offer offer.json] [--skip-eth]

# (no day-7 phase: with --dbc, back → convert → redeem runs in step 2. A devnet registry initialized BEFORE initial converters
#  gets the converter proposed instead; then, 7 days later: rook-devnet accept-converter && rook-devnet dbc-back, same --job)

# the tool alone, any time (status, a floor post, a proof, the report)
ops/rehearsal/sol-devnet/tool/target/debug/rook-devnet --rpc https://api.devnet.solana.com --job ~/rook-devnet/job-1 --payer ~/rook-devnet-payer.json status
```

`--phases` (default `build,deploy,canister,programs,drill,drive,cleanup`) runs a subset; the job directory keeps the state between
runs (a second run on the same devnet lists after the first: `plan` takes the registry's next three listing ids, and the canister's
install argument names their PDAs before they exist). `--keep-canister` leaves the canister for a later `--phases drive`.

`run.sh --devnet` asks `Type 'rehearse'` before it spends cycles (`--yes` skips it). It **always deletes the canister** on exit
(a trap, also on failure or Ctrl-C) unless `--keep-canister`, **without stopping it first** (sol-sign's 2026-10-09 fix: a stopped
canister cannot run the temporary wallet that withdraws its cycles), and retries once after a `start`.

## 4. The checklist and what each step proves

The program half (`rook-devnet`, every step simulated, then signed, sent, confirmed; refusals only simulated, no fee):

| Step | Proves | Pass |
|---|---|---|
| deploy | both v3 binaries load on the cluster (`e_flags` 3 printed) | `solana program deploy` ok; a re-run skips identical bytes |
| init | `init_registry(owner, Signet, [rook_backing's "converter" PDA])` by the upgrade authority; guardian, keeper, treasury, `init_desk(DeskParams)` and `init_module(Params)` with the **launch file's lamports**; three listings; the converter allowed from init (PASS step) | every instruction landed; `status` shows `revenue_target`, the desk's bond and bounds = the launch file's |
| refusals | a second `init_registry`; a `bc1p` custody on a signet registry (`BadInput`); a guardian's frog post (`NotKeeper`); a stranger's `mint_item` (`NotCustodian`); the keeper acting as owner (`NotOwner`) | each refused with that code |
| drill | keeper post; a public mint; `advance` → `return_advance` (the stubbed Relay leg's give-back) → `advance` → `mint_item` with a Merkle proof into the module → `settle_advance`; a second `mint_item`; a `bc1p` redemption refused (`BadDestination`); two redemptions to `tb1p…`; `prove_round` (keeper); `resolve` ×2; `note_sent`; `record_delivery`; `mark_undeliverable`; a burn | each landed or refused as stated; then **the books** (vault − rent ≥ backing + reserve + surplus + revenue) and **the wX ledger** (supply = (held − open) · 10⁹ + eth_backed) read back exact |
| dbc (opt.) | DBC on devnet with rook_backing's claimer PDA as fee claimer; `back`'s DBC CPI runs signed by the claimer, then rook_core `convert` signed by the `["converter"]` PDA (claimer pays); a holder's `redeem_backing` | the coin migrated to DAMM v2; `back` lands and the vault holds wX; the holder receives wX |

The canister half (`drive.py checklist`; B2's eleven items):

| # | Item | How | Pass | Real / stubbed |
|---|---|---|---|---|
| 1 | genesis check | a tick on the Solana home | no refusal: both providers agree devnet's genesis hash (`solrpc::genesis_hash`) | real |
| 2 | simulate before sign, unfunded | the keeper posts both floors, the holder buys (demand), the canister has no SOL; ticks | the line's `error` holds the refused simulation (insufficient funds); **no signature spent**; cycles of that tick recorded | real |
| 3 | funding, the line opens | the payer sends 0.05 SOL to `sol_address` (or a faucet top-up, `--fund-wait`) | `sol_line.line.nonce_account` set, opened by the line itself | real |
| 4 | simulate / sign / land through the nonce line | ticks until an `Advance` outcome | its signature `finalized` on devnet | real |
| 5 | ETH and frog steps on one line | both flows advance | the outcomes hold `Eth.*` and `Frog.*` steps in intent order | real (`SKIP` with `--skip-eth`) |
| 6 | the keeper's floor post | `post_floor` by the keeper key; the canister's `frog_floor_reading` | the post landed; the tick says the registry's keeper is not the canister and drops the reading (D-a) | real (keeper = operator key) |
| 7 | frog advance + `mint_item` with a proof | the frog advance lands; `relay_give_back` → `return_advance` lands; with `--frog-offer`: offer → buy on signet → `mint_item` | Frog.Advance and a GiveBack landed; `mint_item` by the canister only with the offer | advance/return real; **Relay stubbed**; mint real only with a signet purchase |
| 8 | a ticket to a test taproot: `note_sent` / `record_delivery` / `mark_undeliverable` | the canister's custody pass on its own frog's ticket | with `--frog-offer` | otherwise `SKIP`, covered by the program drill on devnet |
| 9 | an upgrade carrying the line | `install --mode upgrade --upgrade-unchanged`, same module and argument | `sol_line` identical, the log says `upgraded: state restored`, the next tick runs | real |
| 10 | `sol_line` and the monitor | `monitor-check.mjs` (the monitor's anonymous query + `solLineAlerts`) | SEV1 "Fund the canister's Solana address" while unfunded (item 2), no alert at rest | real (monitor code, not service) |
| 11 | cycles | `cycles()` around every tick | reported: per tick, and for the refused-simulation tick | measured |

**Verdict**: `== PASS` from `run.sh` (exit 0) when every phase passed: the tool's report has no `FAIL` and `drive.py` has no
`FAIL`. `SKIP` (items 7–8 without an offer) and `INFO` lines do not fail the run; the runbook's reader decides whether the skipped
items need the signet purchase before launch.

## 5. Safety rails

- The tool refuses an RPC whose genesis is mainnet-beta's; `run.sh --devnet` checks devnet's genesis before deploying and refuses a
  launch file naming Bitcoin mainnet; `canister-init.mjs` refuses `key_1` outside a local replica and a Bitcoin network other than
  signet/testnet4.
- No secret is printed or written into the repository: the payer is the operator's file; the role keys are created `0600` in the job
  directory (default `$TMPDIR`, a repository path is refused); the canister's keys are threshold keys.
- The canister's install argument is decoded back with the monitor's IDL before use; the ETH side only reads Ethereum mainnet.

## 6. Results of the local runs, 2026-10-09

**`run.sh --local-validator --dbc`** (`solana-test-validator` 4.1.2, the launch file of that evening: 22.814134924 SOL per ETH
from Coinbase, OKX, Kraken and KuCoin, 19 bps apart): **56 PASS, 0 FAIL**.

- **The SBPF v3 binaries load and run** (rook_core 867,112 bytes, rook_backing 510,712 bytes, ELF `e_flags` 3): deploy, the whole
  init (registry on Signet, treasury, desk 45,628,270 lamports in [11,407,067, 22,814,134,924], three listings with the launch
  params), the five refusals, the whole drill, and the books and wX ledger exact afterwards.
- **DBC against devnet's own Meteora programs** (cloned): `init_backing`, `register_partner`, `create_config` (2 SOL threshold,
  50 % migration fee to the claimer PDA), `register_partner_config`, a launch, `open_vault`, a PartialFill buy to the threshold,
  `migration_damm_v2` (255,886 CU), and `back` simulated: DBC's `withdraw_migration_fee` CPI from the v3 rook_backing ran, then
  rook_core's `convert` refused `NotConverter` — the converter's 7 days (62,135 CU). A test validator's clock cannot be moved
  (`--warp-slot` moves slots, not `unix_timestamp`), so `back` past the gate is the devnet run's day-7 phase.
- v3 CU (whole transaction, incl. the 150-CU compute-budget instruction), against the v2 LiteSVM table of `rook-core-as-built.md` §4:

  | Instruction | v3 (test validator) | v2 (LiteSVM) |
  |---|---|---|
  | `post_floor` | 13,453 | 13,915 |
  | `mint` (+ an idempotent ATA create) | 44,552 | 36,287 |
  | `advance` | 17,309 | 18,062 |
  | `return_advance` | 13,462 | — |
  | `mint_item` (Inscriptions, 2-hash proof, 3 accounts created) | 37,325–41,846 | 41,718 (EthNft) |
  | `settle_advance` | 26,289 | 32,063 |
  | `redeem` | 24,660 | 26,572 |
  | `prove_round` / `resolve` | 8,972 / 11,226–11,521 | — / 11,669 |
  | `note_sent` / `record_delivery` / `mark_undeliverable` | 6,493 / 14,133–17,133 / 6,419 | — / 14,231 / — |
  | `init_registry` / `init_module` / `init_wrapper` | 14,206 / 33,423–37,988 / 30,472–39,493 | — |

**Rerun after the initial-converter change** (rook_core as-built §8.8, rook_backing as-built §7.4; same evening, 22.784 SOL per ETH
from the four sources, 4 bps apart): **59 PASS, 0 FAIL**, `== PASS`. `init_registry(owner, Signet, [rook_backing's "converter" PDA
4ZrpSnfp…VAed])` landed (14,677 CU) and the PDA reads back as an allowed converter (a PASS step); the drill as before; then
DBC: `init_backing` (24,143), `register_partner`, `create_config`, `register_partner_config`, a launch, `open_vault`, the buy, the
migration, a keeper `post_floor` on listing 0 (13,435; `convert` needs a fresh floor), **`back` landed** (80,289 CU on v3: DBC
`withdraw_migration_fee` signed by the claimer, unwrap, rook_core `convert` signed by the `["converter"]` PDA and paid by the
claimer; 121,359,223 wX base units into the coin's vault) and a holder's `redeem_backing` (30,863 CU; 9,708,737 wX base units for a
tenth of its coins). No clock movement was needed: there is no day-7 phase any more (`--phases dbc-back` is gone from `run.sh`; the
tool keeps `accept-converter` / `dbc-back` only for a registry initialized before this change).

**`run.sh --local-replica`** (dfx 0.32 PocketIC, `key_1`): **PASS**. The rehearsal's install argument is accepted (every install
check: the Solana home, `eth_nft` naming rook_core PDAs, signet, the frog wrapper as a PDA); `sol_address`, the taproot custody and
float (`tb1p…`), the Ethereum address; a tick **against real devnet** from the local replica: both providers agreed devnet's genesis
hash and the reads at `finalized` answered ("the frog wrapper does not exist": the programs are not on devnet yet), 1,103,879,969
cycles at local prices; `set_floor_sink` / `set_purchase_limit` (5 SOL) on the frog module; 251 SOL refused over the 250 SOL ceiling
(D-b); an upgrade with `--upgrade-unchanged` restored `sol_line` and `floor_state` identically; the monitor's read of `sol_line` (none
yet: no alert). The buyer module: 8,591,367 bytes, **1,666,004 gzipped (79 % of the 2 MiB single-message install, D-d)**.

Not run here (no devnet SOL spent and no cycles): the devnet deploy and the IC canister's checklist — §3 step 2.

### Devnet runs 3-5, 2026-10-09 (operator's payer, throwaway canisters on IC mainnet, `--skip-eth --frog-floor-sol 1`)

- **Run 3**: 61 PASS, 10 FAIL. (a) `back` reported "not confirmed in 90 s" although it **landed finalized 9 s after it was sent**
  (`kop7XCUb…`, slot 509,341,616, 80,289 CU, vault backed with 121,359,223 wX): the tool polled `getSignatureStatuses` without the
  history on a load-balanced RPC. The tool now waits for the blockhash's real expiry (block height past `lastValidBlockHeight`),
  resends the same bytes, searches the history, and `dbc-back` skips a `back` that already landed; `redeem_backing` then landed.
  (b) The canister's line stalled: OnFinality's public endpoint rate-limits per IP (≈ 1 request/s, HTTP 429 / -32029), and the
  replicas reach IPv4-only providers through shared proxies, so replicas split between 429 and the answer: "No consensus" on 82
  reads in 40 ticks. The nonce account opened only late, the frog advance landed during the give-back wait, and `drive.py` missed
  its outcome (a flow takes its outcomes off `sol_line` at its next pass): `drive.py` now records outcomes after every tick and
  from the tick lines, and gives back in one loop. (c) `burn-units --all` asked for 1,797,304,043 units with one whole unit in
  transit (the deleted canister's advance): rook_core burns only live units (eth_backed − transit), correctly; the tool now caps
  at live units. The books and the wX ledger hold on every listing (`status` prints both now).
- **Run 4**: ZAN's public endpoint answered every replica 429 (agreed `{"http":429}`, so "providers disagree"); stopped.
- **Run 5** (`api.devnet.solana.com` + `devnet.rpcpool.com`: one operator under two names, see `run.sh`): **PASS**: unfunded
  simulation refused and paged SEV1, the line opened its nonce account itself, the frog advance and its `return_advance` landed
  finalized through the nonce line, the upgrade carried the line, 0 alerts at rest; 4 consensus misses in 9 ticks, retried.
  Ticks cost 1.6-20 B cycles on the IC (mean 12.7 B). A launch needs keyed providers.

### After runs 3-5: toward mainnet (2026-10-09)

- **Launch day 1 does not deploy the programs** (decisions D-e, D-f, D-g). The runbook is `launch-day1-solana.md`:
  - `ops/solana/ceremony-day1.sh` reserves the mainnet ids and creates the 12 collection configs plus the genesis ROOK coin. Simulated on mainnet: **0.098384680 SOL**.
  - `ops/solana/deploy-later.sh` deploys once the community fund (`ops/solana/fund-status.mjs`) reaches **9.54 SOL**.
- **The launch file** (`launch-params.mjs`) now carries `meteora` (every `create_config` field with its source) and `listing_order`. Its defaults are mainnet: `--cluster mainnet-beta`, `--bitcoin-network mainnet`. For this devnet kit, keep passing `--cluster devnet --bitcoin-network signet`. `rook-devnet dbc-launch` still uses its own 2 SOL devnet curve.
- **The canister's tick cost** after run 5 was cut by smaller outcall caps; see `launch-day1-solana.md` §6. Run 5's conclusion stands: a launch needs keyed providers. The recommendation is Helius + Ankr, both reachable over IPv6.

## 7. After the run

- The canister is deleted (its remaining cycles withdrawn to the identity's cycles-ledger balance); the SOL at its address stays
  there, worthless.
- The programs stay on devnet, upgradeable by the payer, the registry owned by it: later runs reuse them (`deploy` skips identical
  bytes, upgrades changed ones). `solana program close <id> --bypass-warning -u devnet -k <payer>` returns their ≈ 9.6 SOL **but the
  program id can never be deployed again**, on devnet, by anyone: do it only when the dev keys are retired.
- The holder's units are burned back at the floor by the checklist's last step (`rook-devnet burn-units --listing eth|frog --all`
  by hand otherwise).

## 8. The canister-only rehearsal (W15)

Status: **tool built and tested offline 2026-10-11** (`golive-2026-10-08`); the devnet + IC run is the operator's. It rehearses the
W10 ceremony (`ceremony-canister-only.md`) on devnet with `test_key_1`: **no program is deployed** (`rook_core = "books"`: the
canister keeps the books itself, §12 and §16 of `canister-only-solana.md`); the canister makes every wX mint, every Metaplex metadata
and every DBC config itself, in the unlock order, as `K_main`'s float covers each. Same rails as §5: devnet only (a mainnet provider
is refused at the parse, the genesis checked before anything is created), the canister a throwaway on the IC deleted by the trap
unless `--keep-canister`, the payer the operator's file outside the repository, read by the solana CLI only; nothing printed.

### Commands

```sh
# the tests (offline; the monitor's dependencies installed once: cd monitor && npm ci)
node --test ops/rehearsal/sol-devnet/test/*.test.mjs

# 0. a dry run: every command printed, none run (no replica, no devnet, no IC)
ops/rehearsal/sol-devnet/run.sh --books --local-replica --no-build --dry-run

# 1. free: the local replica (PocketIC's key_1), devnet READS only
(cd ops/rehearsal/sol-devnet && dfx start --clean --background)
ops/rehearsal/sol-devnet/run.sh --books --local-replica [--count 2]       # install, addresses, pins, the unfunded wait, views, upgrade
ops/rehearsal/sol-devnet/run.sh --books --local-replica --payer ~/rook-devnet-payer.json   # + the float sent: the local canister opens on devnet
(cd ops/rehearsal/sol-devnet && dfx stop)

# 2. the rehearsal: a throwaway canister on IC mainnet (test_key_1), the registry's first five, one intent end to end
ops/rehearsal/sol-devnet/run.sh --books --devnet --payer ~/rook-devnet-payer.json --next-to <buyer canister id> \
  --count 5 --intent [--evidence ~/rook-devnet/bayc-evidence.json] --job ~/rook-devnet/books-1
```

`--count 2` (the default: BAYC and Milady, ≈ 0.1 SOL) keeps the devnet SOL small; `--count 5` pins the registry's first five (BAYC,
Milady, Mad Lads, SMB Gen2, Bitcoin Frogs; ≈ 0.17 SOL; the Solana pins with `--sol-custodian` = the canister's `sol_address`). Mainnet
pins 24 (every working collection, operator decision 2026-10-12); `--count` takes 1 to 24. The `pins` phase also calls
`coins_request_open` for each pinned collection (a collection opens on its first request; auto-open is off). `--phases build,canister,pins,fund,open,checklist,cleanup` runs a subset on the same
`--job` (`--keep-canister` on the first run); `--open-wait` (1800 s) bounds the watch; `--yes` skips the `rehearse` prompt.

### Phases and what each proves

| Phase | Does | Proves |
|---|---|---|
| build | the wasm (`cargo build --locked --release --target wasm32-unknown-unknown`), gzipped (2.33 MB: over the 2 MiB single-message limit) | — |
| canister | `canister-init.mjs --books` → `init.hex` (decoded back with the monitor's IDL); create on the subnet (`--next-to` / `--subnet` / `--subnet-type`); **chunked install** (`autobuyer/scripts/chunked-install.sh --mode install`: 1 MiB pieces, each hash checked, the module hash read back); `books.py addresses` → `canister.json` | the W10 install argument (`home = Solana { cluster devnet, rook_core "books" }`, `wrapper = module = "books"`, no frog wrapper) is **accepted by the canister**; `sol_address` (`K_main`), `sol_address_of("claimer"\|"treasury"\|"coll:i")`, `evm_address`, `custody_address` derive from `test_key_1` |
| pins | `cargo run --example launch_pins -- --evm-custody <evm_address> --btc-custody <custody_address> --count n --uri-base … --out <job>/pins`; `coins_pin --argument-file <index>-<slug>.args` each | each pin is accepted (checked as the canister checks it); `coins_collections` reads them `Pinned` |
| fund | the float printed with `K_main`; with `--payer`: `solana transfer` from the operator's keypair, else the operator sends it; the balance polled (`--fund-wait`) | `K_main` holds the float |
| open | `tick`s until every pinned collection is `Open` (bounded, `--open-wait`); every wait's reason printed (the tick's `coins:` lines, `coins_log`'s waits); then `coins_collections`, `books_collections`, `books_view(i)`, `treasury_status`, `desk_status`, `pool_desks` into `views.json` | the canister signs with the threshold Ed25519 key and lands, per collection, the wX mint + metadata transaction and the DBC config transaction on devnet, reads both back and lists the collection in its books |
| checklist | `books.py checklist`, the report in `<job>/books-report.json` (PASS / FAIL / INFO / SKIP each) | below |
| cleanup | the trap: `dfx canister delete` (no stop first, sol-sign's fix), the cycles withdrawn | — |

The checklist's items:

| # | Item | Pass |
|---|---|---|
| 1 | genesis | a tick on the books home: both providers agree devnet's genesis hash (a disagreement refuses the whole tick) |
| 2 | unfunded | with the pins in and `K_main` empty the opening **waits** (`coins: collection 0: it costs … K_main holds …`), nothing landed, no signature seen; the tick's cycles recorded |
| 3 | funded | `K_main` ≥ the float; the crank line's durable-nonce account read in `sol_lines` (INFO until the first signed step opens it) |
| 4 | each collection Open | its two opening transactions finalized on devnet (the signatures from the tick lines) and read back: the wX mint and the DBC config exist at the pinned addresses |
| 5 | `books_view(i)` | listed in `books_collections` with the pinned mint; `module = sol_address_of("coll:i")`; `expected_supply` 0 before any mint; owed = backing + reserve + surplus + revenue; nothing held; not halted |
| 6 | one intent end to end (`--intent`) | `open_intent(Mint { coll 0, recipient = the payer's address })`, `min_deposit` sent from the payer to the intent's address, `intent_status` polled to `Done`, the wX at the wallet (`getTokenAccountsByOwner`), `books_view.sol_backed` moved by `minted_units`. **Needs a fresh floor**: the runner's `eth_floor_evidence` (`ROOK_HOME=books`, an OpenSea key, pointed at the throwaway), or `--evidence <json>` (`{ "collection": "0x…", "listings": ["<fulfillAdvancedOrder calldata hex>", …] }` as the runner builds them); else `SKIP` |
| 7 | the monitor | `monitor-check.mjs --books`: the four views the monitor pages + every `books_view` through `monitor/src/ic.mjs`; `books.mjs`'s alerts: none SEV1; `desk_status` with no halt and no unexplained memo; `treasury_status` with no wait and the drand relays agreeing |
| 8 | an upgrade carrying the state | `chunked-install.sh --mode upgrade` with the same module and argument: `coins_collections` and `books_collections` identical, `upgraded: state restored` in the log, the next tick runs |
| 9 | cycles | per tick (INFO) |

Then the site: `VITE_CANISTER_ID=<the kept canister> VITE_IC_HOST=https://icp-api.io` (W12) shows the intent desk and the Reserves
page against it (`--keep-canister`; delete by hand after). Local-replica runs stop at item 2 without `--payer` (3–6 `SKIP`), and run
items 3–5 on devnet with it (the local canister's key signs; the float is real devnet SOL).

### What the operator brings

- **A payer keypair with devnet SOL**, outside the repository (`solana-keygen new -o ~/rook-devnet-payer.json`; `solana airdrop 2 -u
  devnet -k ~/rook-devnet-payer.json` or <https://faucet.solana.com>). Devnet SOL: the float `0.05` (the reserve the money flows keep)
  `+ 0.0226 × n` (each opening: mint rent 0.0015, metadata rent 0.0051, the 0.01 Metaplex fee, config rent 0.0060, fees) `+ 0.0015`
  (the nonce account), rounded up: **0.10 SOL for two, 0.17 SOL for the five** (≈ 0.2 SOL with margin); `--intent` adds the
  intent's minimum **0.05 SOL + 0.0015 fee** from the payer. The SOL at a deleted canister's addresses stays there, worthless.
- **A dfx identity with cycles** on the cycles ledger: the creation fee (500 B on 13 nodes, ≈ 1.3 T on 34) plus the ticks (≈ 2.5 B of
  Solana outcalls a quiet tick, 1.6–20 B measured in run 5) and the signatures (≈ 26 B each; 2 per collection, 2–3 for an intent):
  `--cycles` (3 T) is what the canister is created with, the rest withdrawn at the delete. **≥ 3.5 T** on the ledger.
- **The subnet**: `--next-to <the buyer's canister id>`, `--subnet <id>` or `--subnet-type <type>`.
- The toolchain of §2 (dfx 0.32, solana 4.1.2, the canister's wasm32 target, node ≥ 22 with `monitor/` installed, python3); the
  launch files (`deployments/launch/nft-pilot.json`, `graduation-tiers.json`) at the commit; `node solana/tests/q1/design_curve.mjs`
  for the pins' curves.
- For item 6, a fresh floor on BAYC: the runner with an OpenSea key, or an evidence file.

### Not tested here (needs devnet SOL and IC cycles)

The chunked install and the openings against the IC, the intent, the monitor read of a live canister, the cycles per tick: the
operator's run writes them into `books-report.json`; §6 is the place for its results.

### Results, 2026-10-12: the local replica, real devnet transactions (`--local-replica --payer`, 2 collections)

Four funded runs (`~/rook-devnet/books-local-{1..4}`), the operator's devnet payer, the canister on the local replica (PocketIC's
`key_1`), every transaction on devnet. **Run 4 passed every item it runs: 23 PASS, 0 FAIL** (items 2 and 6 skipped: the float was
already sent, and `--intent` was not given). It also covered the Magic Eden M2 merge.

- **Run 1** found the one canister change: DBC now writes its config at **layout version 1**, not 0 as DBC 0.2.1 did. Read field by
  field on devnet, it is the same 1,048-byte layout, and every field Rook pins is identical. `solix2::guard::check_config` now
  accepts 0 or 1 and refuses anything higher (`tests/solix2_builders.rs`).
- **Runs 2 and 3** opened both collections on devnet. The rest of their FAILs were the rehearsal tool's:
  - dfx's JSON writes a candid tuple as `{"0": …, "1": …}`, and a multi-value return as one document per value.
  - A request taken at once is `Requested`, not queued.
  - The canister's own timer can hold the tick.
  - Each phase now answers for its own steps.
  - The canister refuses an upgrade while a tick runs: the rehearsal now stops it first and starts it after, as the timelock's
    upgrade must.
- **Run 4.** Both collections opened from one request each:
  - wBOREDAPE: mint `CRVKumkDSnk8eSeCoFuRvzjyyzo3mYmGDAhnbL7cjSjQ`, config `CvG4LW8PgRw1kLLiDKRLZRXRrcBeGRFVz7h7A4rdB5Sv`.
  - wMILADY: mint `27JkTCUjFD9StvPVcW7nVNaDv8ft9WSD2zG3NKtiyztJ`, config `2J5YUfvFrQFiXKCaWKwWv1P3bfjseBS6ALkYAM2vtmwN`.

  The opening transactions are finalized and read back. `books_view` is consistent for both, and the monitor raised no alert. The
  chunked upgrade carried the state, and the canister ticks on after it. The two openings and the nonce account took ≈ 0.043 SOL of
  the 0.1 SOL float.

| Cycles per tick (run 4, 11 ticks) | |
|---|---|
| min | 0.16 B |
| mean | 30 B |
| max | 130 B (a tick that signs an opening) |

Not covered yet: an intent end to end (`--intent`, needs a fresh floor), and the run on IC mainnet (`--devnet`, a throwaway
canister with `test_key_1`, the operator's cycles).

### Next: the fiduciary run (decides O-13, the RPC tier)

The run on the IC, on the chosen subnet, with the chosen providers' devnet hosts. It shows whether Helius Free serves 34 replicas
calling at once. It needs ≈ 3.5 T cycles on the operator's dfx identity and a Helius Free key made for this run only.

```
! ops/rehearsal/sol-devnet/run.sh --books --devnet --payer ~/rook-devnet/payer.json --count 2 \
    --subnet pzp6e-ekpqk-3c5x7-2h6so-njoeq-mt45d-h3h6c-q3mxf-vpeq5-fk5o7-yae \
    --rpc-urls "https://api.devnet.solana.com,https://devnet.helius-rpc.com/?api-key=$HELIUS_DEVNET_KEY" \
    --job ~/rook-devnet/books-fiduciary-1
```

PublicNode serves mainnet and testnet only (its devnet hosts answer 404, checked 2026-10-12), so the run pairs Helius with the
Foundation's devnet host. That host has no IPv6: the replicas reach it through the IC's IPv4 proxies, so a 429 from it says nothing
about mainnet. Read the tick lines for provider errors (HTTP 429, "providers disagree"). None over the run: Helius Free at launch. Any: Helius
Developer from the ceremony.

**Run, 2026-10-12 (`~/rook-devnet/books-fiduciary-2`, canister `gbf5z-zyaaa-aaaar-qch5q-cai` on `pzp6e`, then deleted): PASS, 23/0.**
Helius Free (devnet) with `api.devnet.solana.com`: no HTTP 429, no rate-limit error, no provider disagreement in any tick line.
Both collections opened from one request each (wBOREDAPE mint `5pNE5tY79BE5LSnLC4i7bctPK7HdweuRAMMLV3qeZSPJ`, wMILADY mint
`3ks47bkyD87vrcZBepWgQ7KHcL55VEKFPbmTYw2mfeSV`), books consistent, the upgrade carried the state. The canister's own timer ran
ticks beside the rehearsal's, so the per-tick cycle figures (mean ≈ 49 B, max ≈ 361 B) overlap and are rough.

Verdict for O-13: **Helius Free at launch**. The run is short (≈ 9 rehearsal ticks plus the timer's), so it shows the burst of 34
replicas is served; the monthly 1 M credit cap (≈ 2.1 M requests at rest on 34 nodes) is still expected to run short. Watch Helius's
usage page from the ceremony, and move to Developer from fees before the cap.

### Next: the intent end to end (item 6)

Free (the local replica, the devnet payer). Collection 0's floor must be fresh first: with `OPENSEA_API_KEY` in the environment, the
rehearsal builds the evidence itself (`evidence.ts`: the runner's own encoding of BAYC's cheapest Seaport 1.6 listings for the
canister's EVM address; the canister checks them against Ethereum mainnet). The key is read from the environment only.

```
! OPENSEA_API_KEY=<the rotated key> ops/rehearsal/sol-devnet/run.sh --books --local-replica --payer ~/rook-devnet/payer.json \
    --count 2 --intent --open-wait 1800 --job ~/rook-devnet/books-intent-1
```

**Run, 2026-10-12 (`~/rook-devnet/books-intent-1`, local replica, devnet): PASS, item 6 included.** The rehearsal's first try found
the floor-evidence bug (OpenSea's best listings carry no signature; fixed in `4455c51`, the handoff §8). With signed evidence the
canister refused none of 8 BAYC listings, scanned Ethereum's sales (only once its cycles were over 8 T) and posted a floor of
≈ 143.7 SOL a wBOREDAPE. Then: intent 0 opened, 0.05 SOL deposited, `Done`, 327,073 base units of wBOREDAPE minted
(`rook:v1:0:mint:1:i0`) and at the payer's wallet, `books_view` moved by exactly that (`sol_backed`, `expected_supply`).
