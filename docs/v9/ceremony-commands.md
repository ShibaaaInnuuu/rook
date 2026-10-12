# Ceremony day: the commands, in order

The exact shell commands for the canister-only launch ceremony (`docs/v9/ceremony-canister-only.md`) on the IC mainnet fiduciary
subnet `pzp6e-ekpqk-3c5x7-2h6so-njoeq-mt45d-h3h6c-q3mxf-vpeq5-fk5o7-yae`, with the operator's dfx identity `default` (the cycles are on
its cycles ledger account). The order is the EVM-era runbook's (`docs/deploy/canister-mainnet.md`, "What goes on the IC, in order") on
the books home: build, create both canisters, install the buyer, read its addresses, install the timelock on the buyer's id, hand the
buyer over, blackhole the timelock, then the operator's acts (pins, float, ops key).

Marks: **[CYCLES]** spends cycles from the ledger account (gone once spent). **[IRREVERSIBLE]** cannot be undone, or only through a 72 h
timelock proposal. **[FUNDS]** moves real money on another chain. Unmarked commands only read or write local files.

Every command runs from the repository root unless it says `cd`. `B` and `T` are the buyer's and the timelock's canister ids, set in §3.

## 0. Before (nothing on the IC)

```sh
git checkout <the ceremony commit> && git status                 # clean
cd autobuyer && scripts/reproducible-build.sh && cd ..           # Docker, linux/amd64: build/buyer.wasm(.gz), timelock.wasm + SHA-256s
#   a second machine, same commit, same command: the same hashes. Record them with the commit (deployments/canister-ic.json).
#   Install buyer.wasm.gz (≈ 1.88 MiB, chunked) and timelock.wasm. NEVER timelock-rehearsal.wasm (10 s floor).
(cd ops && npm ci) && (cd monitor && npm ci)                     # the tools' dependencies
(cd ops && npm test)                                             # ops/test/ceremony_init.test.ts among them

dfx identity use default
dfx identity get-principal                                       # the deployer (a temporary controller until §5)
dfx cycles balance --network ic                                  # 20.59 T expected (budget in §3)
```

Re-read the subnet on the IC dashboard (`https://dashboard.internetcomputer.org/subnet/pzp6e-ekpqk-3c5x7-2h6so-njoeq-mt45d-h3h6c-q3mxf-vpeq5-fk5o7-yae`):
type fiduciary, 34 nodes, accepting canisters.

Read the five role principals back (ceremony-canister-only.md §0, O-7) and compare them with `ops/ceremony/timelock-init.mjs`
(`PROPOSERS`, `GUARDIANS`); the timelock refuses a principal named twice or in two roles, and the tool refuses a typo (checksum):

```sh
dfx identity get-principal --identity <p1 identity>   # jwtyr-bjz34-xpk3j-guwcn-igu4r-cdpw5-cybm6-xx5lt-nvrcm-qvlka-fqe
dfx identity get-principal --identity <p2 identity>   # jd5fp-gbdia-l5jyl-l64ph-nuwbd-sltlv-ruwnv-gxpyf-7yofw-ba5zw-sqe
dfx identity get-principal --identity <g1 identity>   # 4423w-dirhd-6mmzf-zrn3b-ukl6j-3p4mb-xhljt-gu67m-zojhm-yqdwk-fae  (the pause guardian)
# g2 ght7l-5bmui-5xas3-jjwzi-xm4ac-g57wd-hby3u-su46d-mefmb-ehiua-mae and g3 5re2m-u7frt-k642z-nzj7d-kskmn-67zi5-ewpsv-ghh7h-3bqkl-f5tz6-oqe
# are offline: read them from their devices.
dfx identity get-principal --identity <runner identity>   # the runner's OPERATOR_IDENTITY_PEM: the install's `operator`
```

The runner's identity must also be on this machine: after the handover (§5) only it (the `operator`) can make the operator calls of §6.

## 1. The install arguments (local, live reads only)

```sh
cp deployments/launch/ceremony-input.example.json deployments/launch/ceremony-input.json   # git-ignored: it will hold the Helius key
$EDITOR deployments/launch/ceremony-input.json     # operator = the runner's principal; the Helius URL; nonce_seed (PERMANENT)
mkdir -p ops/ceremony/out                          # git-ignored
node ops/ceremony/mainnet-init.mjs deployments/launch/ceremony-input.json --bin ops/ceremony/out/buyer-init.bin \
  > ops/ceremony/out/buyer-init.hex
```

The tool refuses any placeholder, test key, test network, rehearsal seed, unmeasured Ethereum provider or non-esplora explorer; it
reads live that both Solana providers answer mainnet-beta's genesis, both Ethereum providers chain 1, both explorers the 900,000
checkpoint, and that Seaport 1.6's and OpenSea's SignedZone's code hashes on Ethereum are the same on both providers AND the ones
`docs/v4/eth-nft-canister.md` recorded (`0x74499ac0…aec0`, `0x9e6930be…e324`). Read the summary it prints on stderr: every value
and where it comes from (query strings redacted; the hex itself carries the Helius key, as O-13 decided).

The dry run of exactly this argument on a local replica (no cycles; stops the replica at the end):

```sh
(cd autobuyer/canister-rs && cargo build --locked --release --target wasm32-unknown-unknown)
(cd autobuyer/timelock-rs && cargo build --locked --release --target wasm32-unknown-unknown)
ops/ceremony/local-dry-run.sh deployments/launch/ceremony-input.json     # "DRY RUN PASSED"
```

It installs the `--local-replica-dry-run` form (key_name swapped to `dfx_test_key`, nothing else, and says so), then reinstalls the
exact production bytes (`key_1`: dfx 0.32's local replica serves only keys named `key_1`), installs the timelock on the buyer's id,
hands the buyer over and checks the operator still operates and the former controller does not.

## 2. Cycles budget

Creating a canister on the 34-node subnet costs ≈ 1.31 T (500 B × 34 / 13), taken out of `--with-cycles`; the cycles ledger also
charges 0.1 B per creation. From 20.59 T:

| | `--with-cycles` | left in the canister | why |
|---|---|---|---|
| timelock | 2.3 T | ≈ 0.99 T | over its own alert threshold (`LOW_CYCLES` 0.5 T); it spends almost nothing; anyone can top it up once blackholed |
| buyer | 18.0 T | ≈ 16.69 T | over the canister's 8 T Ethereum sale-scan floor (ceremony §0) with ≈ 8.7 T, ≈ 4 quiet days at ≈ 2 T a day |
| left on the ledger | | ≈ 0.29 T | margin; installs and calls are paid by the canisters, not the ledger |

Top the buyer up before it nears 8 T (`dfx cycles top-up --network ic $B <amount>`, anyone may): the monitor's low-cycles alert is
the cue (ceremony §0).

## 3. Create the two canisters

```sh
cd ops/ceremony                                   # dfx.json: buyer = ../../autobuyer/build/buyer.wasm.gz, timelock = …/timelock.wasm
dfx canister create buyer    --network ic --subnet pzp6e-ekpqk-3c5x7-2h6so-njoeq-mt45d-h3h6c-q3mxf-vpeq5-fk5o7-yae --with-cycles 18000000000000   # [CYCLES] [IRREVERSIBLE]
dfx canister create timelock --network ic --subnet pzp6e-ekpqk-3c5x7-2h6so-njoeq-mt45d-h3h6c-q3mxf-vpeq5-fk5o7-yae --with-cycles 2300000000000    # [CYCLES] [IRREVERSIBLE]
B=$(dfx canister id buyer --network ic); T=$(dfx canister id timelock --network ic); echo "buyer $B timelock $T"
dfx canister info --network ic $B; dfx canister info --network ic $T          # Controllers: the deployer; no module yet
dfx cycles balance --network ic                                                # ≈ 0.29 T
```

`ops/ceremony/canister_ids.json` now holds both ids: commit it. The buyer's id fixes its threshold keys and so every address it will
ever have; a canister deleted here is gone with its cycles.

**§3 to §6 run in `ops/ceremony`** and name the canisters `buyer` and `timelock`: its `dfx.json` gives dfx the two `.did` files. The
buyer's module carries no `candid:service` metadata, so a call made elsewhere would have dfx guess the argument's types (a `coins_pin`
record with them guessed is a different argument).

## 4. Install the buyer, read it back

```sh
../../autobuyer/scripts/chunked-install.sh --network ic --mode install $B ../../autobuyer/build/buyer.wasm.gz out/buyer-init.bin   # [CYCLES] (small: chunk store, install)
#   prints "MATCH: <B> runs the module of …buyer.wasm.gz"
(cd ../../autobuyer && scripts/reproducible-build.sh verify $B buyer.wasm.gz ic)   # MATCH with the build of §0
dfx canister update-settings --network ic buyer --freezing-threshold 7776000          # 90 days (canister-mainnet.md "Budget"); controller-only: now or never
dfx canister call --network ic buyer log --query                                      # "installed"
dfx canister call --network ic buyer eth_nft_config --query                           # chain 1, the 11 collections, "books", relay 300
dfx canister call --network ic buyer cycles --query
dfx canister call --network ic buyer paused --query                                   # false
```

A wrong argument found here is fixed by a reinstall, still possible until §5 (`chunked-install.sh --mode reinstall`, same id, same
keys, same addresses). After §5 only a 72 h timelock upgrade changes anything, and never `key_name`, `btc_network`, the home's
cluster or nonce seed (identity: kept from the install).

The addresses (operator-only methods: the deployer may call them while it is a controller; after §5 use `--identity <runner identity>`):

```sh
dfx canister call --network ic buyer sol_address                   # K_main: the float's address and the Solana collections' custodian
dfx canister call --network ic buyer sol_address_of '("claimer")'  # K_fc
dfx canister call --network ic buyer sol_address_of '("treasury")' # K_tre
dfx canister call --network ic buyer evm_address                   # the Ethereum custody (also Robinhood Chain's, later)
dfx canister call --network ic buyer custody_address               # the taproot custody (bc1p…)
for i in $(seq 0 37); do echo "coll:$i $(dfx canister call --network ic buyer sol_address_of "(\"coll:$i\")")"; done
(cd ../../autobuyer/canister-rs && cargo run --example sol_keys -- --canister $B --key key_1)   # the same addresses, derived offline: compare
```

Write them, with the ids and module hashes, into `deployments/canister-ic.json` and commit.

## 5. The timelock, the handover, the blackhole

```sh
node timelock-init.mjs $B > out/timelock-init.hex                            # target = B; p1 p2 / g1 g2 g3; veto 2; 259,200 s; rules.rs checks
dfx canister install --network ic timelock --mode install --wasm ../../autobuyer/build/timelock.wasm \
  --argument-type raw --argument "$(cat out/timelock-init.hex)"                                       # [CYCLES] (small) target and delay fixed for good
(cd ../../autobuyer && scripts/reproducible-build.sh verify $T timelock ic)                            # MATCH (72 h floor)
dfx canister call --network ic --identity anonymous timelock status --query     # target = B, the five roles, veto 2, delay 259_200, rehearsal_build = false
dfx canister call --network ic --identity anonymous timelock cycles --query

dfx canister update-settings --network ic buyer --set-controller $T          # [IRREVERSIBLE] the handover: answer the prompt "yes"
dfx canister info --network ic $B                                          # Controllers: T, and nobody else

dfx canister update-settings --network ic timelock --freezing-threshold 7776000
dfx canister update-settings --network ic timelock --remove-controller $(dfx identity get-principal)   # [IRREVERSIBLE] the blackhole (canister-mainnet.md step 7)
dfx canister info --network ic $T                                          # Controllers: (none)
```

Do the blackhole in the same sitting as the handover (between them the deployer still controls the timelock). After it the deployer
controls nothing. From here: an upgrade is `propose_staged_upgrade` (or `propose_upgrade`) by p1 or p2, 72 h public, vetoed by any 2 of
g1 g2 g3 (`autobuyer/scripts/timelock-propose-chunked.sh`).

**The guardian.** The buyer has no guardian field in its `Init`: its pause guardian acts through the timelock, which forwards only
`set_paused(true)`. g1 (or g2, g3) pauses the buyer at once, alone:

```sh
dfx canister call --network ic --identity <g1 identity> timelock pause_target     # any ONE guardian; stopping is always safe
dfx canister call --network ic --identity <gN identity> timelock resume_target    # resuming takes 2 of the 3 guardians' votes
```

The operator may also pause (`set_paused(true)`, operator-only) but never resume. Nothing to set on ceremony day beyond the
timelock's install: check `status` lists g1 among the guardians. Do not drill the pause on mainnet unless decided (it freezes custody's
outflow until two guardians resume).

## 6. The operator's acts (identity: the runner's)

`R="--network ic --identity <runner identity>"`. These are operator-only and stay callable after the handover.

**The pins** (ceremony §2; deploy the site's `wx/` metadata first and check `curl -s https://userook.fun/wx/bayc.json`):

```sh
(cd ../../autobuyer/canister-rs && cargo run --example launch_pins -- --evm-custody <evm_address> --btc-custody <custody_address> \
    --sol-custodian <sol_address> --uri-base https://userook.fun/wx/ --out ../../deployments/launch/canister-only)   # "38 pins written … 0.858 SOL were all 38 to open"
for f in ../../deployments/launch/canister-only/[0-3][0-9]-*.args; do               # still in ops/ceremony (its dfx.json types the argument)
  n=$(basename "$f" | cut -c1-2); [ "$n" -le 37 ] || continue
  echo "$f: $(dfx canister call $R buyer coins_pin --argument-file "$f")"            # [IRREVERSIBLE] each index is taken once
done
dfx canister call --network ic buyer coins_collections --query                        # 38 × Pinned; nothing opens unasked (auto-open off)
```

Review `deployments/launch/canister-only/pins.json` before the loop, and commit the directory after.

**The float** (ceremony §3): ≈ 0.2 SOL to `K_main` (`sol_address`), from the operator's own Solana wallet:

```sh
solana transfer --url mainnet-beta <K_main> 0.2 --allow-unfunded-recipient          # [FUNDS] [IRREVERSIBLE]
solana balance --url mainnet-beta <K_main>
```

**The treasury's ops key** (ceremony §5: the fueler's Solana hot key):

```sh
dfx canister call $R buyer treasury_set_ops '("D1EDmAdkmCuPx2QV3k4LHZrPbUsBmvEr69KW1LDUbuuT")'   # the first takes effect at once; a change waits the books' 24 h
dfx canister call --network ic buyer treasury_status --query
```

Then ROOK (ceremony §4, Meteora's own tooling, `fee_claimer = leftover_receiver = K_fc`), the services' canister ids (runner, fueler,
indexer, monitor) and the site (ceremony §6).

## 7. After the ceremony: Robinhood Chain

Not in this Init: the 4663 `evm_chains` entry (Seaport 1.6's code hash read back on 4663, never Ethereum's) and its three collections
come by a timelocked upgrade whose argument is this one plus those (ceremony §1-§2); `custody_caps` stays null.

## What the ceremony's Init holds, and why

`node ops/ceremony/mainnet-init.mjs` prints it with every source; in short:

- `key_1` (`accept_production_key_name` true), Bitcoin `mainnet`, `home = Solana { mainnet-beta, the two input providers, "books",
  the input nonce seed, null, null, null }`, `operator` the runner.
- `evm_chains = [Ethereum]`: the two measured keyless providers, bucket 8, Seaport 1.6 and its code hash read live, max fee 50 gwei,
  priority 1 gwei, gas cap 400k, OpenSea's fee recipient, `max_fee_bps` 300, `min_order_life_s` 600, OpenSea's SignedZone pinned by
  its code hash read live, Limit Break's two transfer validators. **Restricted (SignedZone) listings are in practice skipped**: a zone
  signature lives ≈ 280 s and `min_order_life_s` 600 refuses it; open listings (99 % of Ethereum fills) are bought.
- `eth_nft`: chain 1, the 11 Ethereum collections of the launch set (`coins::unlock::ORDER[..38]`) with `nft-pilot.json`'s
  `maxPurchaseWei` and 500 bps premium, `wrapper = module = "books"`, Relay 300 bps, deposits off, default floor, gas pot, log span,
  no dormancy (every collection active).
- `frog_funding` Relay 400 bps without a float (`nft-pilot.json`), `custody_caps` and `outflow_limits` null (stage 0), no wrapper, no
  holder deposits, no bridge; the EVM home fields (unused on the Solana home): chain 4663, Robinhood's two keyless providers, the
  placeholder vault `0x…01`.
