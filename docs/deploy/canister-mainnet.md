# The buyer canister and its timelock on the IC: the go-live runbook

Written 2026-09-23 at `golive-prep` (merged with `main` at `01a1e5d`). Nothing here has touched the IC mainnet: no canister was
created, no cycles spent, no key used. The install and the handover were rehearsed on the local replica by
`autobuyer/timelock-rs/rehearse.sh` (all seven steps pass, `docs/v2/timelock-controller.md`, "Results"), and the watcher by
`scripts/timelock-watch.test.mjs`.

**Read "Blocked today" first.** Four things in the code or the world stop a mainnet install as the code stands; the rest of this
document is the procedure for the day they are closed.

## Blocked today

1. ~~**The buyer cannot be installed for Bitcoin mainnet.**~~ **Closed 2026-09-25** (`df2f13c`, Phase-1 K2). `KNOWN_NETWORKS` is
   `["signet", "testnet4", "mainnet"]` (`autobuyer/canister-rs/src/canister.rs`, spelled once as `BITCOIN_MAINNET`, the string
   `ordnet::providers()` and the runes oracle match), and `btcsend::checkpoint("mainnet")` is block 900,000. How the checkpoint was
   chosen and checked is below ("The mainnet checkpoint", "The published C7 check"). Every `rehearse_*` update method still refuses
   an install for 4663 or Bitcoin mainnet.
2. **There is only one RPC provider for 4663 that a canister can reach.** The buyer requires at least two distinct `rpc_urls` and
   believes an answer only when all agree (`rpc()`, `rpc_view()`). `https://rpc.mainnet.chain.robinhood.com` answers. The explorer's
   RPC, `https://robinhoodchain.blockscout.com/api/eth-rpc` (where `explorer.mainnet.chain.robinhood.com` redirects), answers a
   **Cloudflare browser challenge** (seen 2026-09-23), which an IC outcall can never pass. The options, in order of preference:
   - **run a Robinhood Chain (Nitro) full node** with a public HTTPS endpoint: an independent second provider you control;
   - ask Robinhood or Blockscout to exempt `/api/eth-rpc` from the challenge;
   - a keyed commercial provider — but **an API key in `rpc_urls` is public**: the install argument is readable and the timelock
     publishes every upgrade's argument in full for 72 hours. A key there is a key anyone can use.
   Both current hosts publish IPv6 (AAAA) records, so the IC's network is not the obstacle.

   **Update, 2026-09-23: keyless second providers exist, and reads are pinned** (`docs/v2/pinned-reads.md`). At 0.104 s blocks,
   a read at `"latest"` is a different question on every IC node and every provider and can never reach consensus, so every state
   read now names an agreed pin (heads rounded to 1,024 blocks, one bucket below the lowest, that block's hash agreed by all) —
   ≈ 107–214 s behind the head. That needs providers that keep state that deep. Measured read-only:

   | Provider | keeps state back to | pinned reads |
   |---|---|---|
   | `https://rpc.mainnet.chain.robinhood.com` (Robinhood) | ≈ 6,220 blocks (≈ 647 s) | yes |
   | `https://robinhood.drpc.org` (dRPC) | ≥ 20,000 (archive) on 2026-09-23; **none from 2026-09-24** ("Unknown state") | yes then; **no** now |
   | `https://rpc.ordofi.network` (ordofi) | ≥ 20,000 (archive) | yes |
   | `https://robinhood-rpc.publicnode.com` (publicnode) | ≈ 92 blocks (≈ 10 s); deeper needs a paid token | **no** |

   **Re-measured 2026-09-26 (audit D, H-1; `docs/audit/2026-09-26/fixes-D-canister.md` §0).** Robinhood serves state
   **6,201** blocks behind its head and not 6,250; ordofi serves 200,000 (archive). `finalized` is **8,096–12,023** behind the
   head on both, in steps of ≈ 3,700–3,900 blocks every ≈ 6.3 min, so no state read at the finalized block is possible on this
   pair. ordofi refuses `eth_getLogs` over more than **10,000** blocks (`-32005`); Robinhood took 8,000,001. The canister now
   reads state only at the fast pin and takes finality from the wrapper's events, paged 8,000 blocks at a time; see
   `docs/v5/custody-hardening.md` H9. **Before go-live, re-measure these four numbers**: a provider that shortens its state
   window below ≈ 2,500 blocks, or its log span below 8,000, breaks the delivery path again (the fakes in `tests/fakerpc` hold
   today's values).

   **Superseded 2026-09-24/25 (docs/v6/phase1-scope.md §3.2 C8, §7 item 7): dRPC is dead for this use.** Measured on 2026-09-24, `robinhood.drpc.org` refuses every state read at an explicit block ("Unknown state. First available state is 1"), and the canister reads only at pinned historical blocks (`pin.rs:123`). Install with `rpc_urls = vec { "https://rpc.mainnet.chain.robinhood.com"; "https://rpc.ordofi.network" }` (Robinhood keeps ≈ 6,220 blocks; ordofi is archive; both publish AAAA records). The measurements below are kept as they were taken.

   ~~Install with `rpc_urls = vec { "https://rpc.mainnet.chain.robinhood.com"; "https://robinhood.drpc.org" }`~~ (as written on
   2026-09-23, superseded above); every provider
   must agree, so two is the fewest that is a check and the most that stays live. ordofi was the spare, swapped in by an upgrade
   argument (public for the timelock's delay). On a local replica against mainnet, 14/14 pinned reads agreed with Robinhood + dRPC
   and 12/12 with all three; publicnode failed 2 of 3. Not yet shown: that these providers answer IC nodes (all sit behind
   Cloudflare), and how often a real subnet's nodes straddle a 1,024-block boundary (a failed read, retried next tick).
3. ~~**The operator key is a custody key.**~~ **Closed 2026-09-23** (`docs/v2/trust-points.md` §1). Every order at
   `open_purchase`, and every quote when recorded (by the loop or `record_quote`) and again before anything is signed, is checked
   against the chain: a rune listing of this buyer's vault, not paused; its rune; the scriptPubKey of its `btcVault`; our own change
   script; at least the vault's announced units; the vault purchase `Pulled`; and a spend of at most the released ETH's value at
   the reference rate plus 3% (`rates::cap_sats`, the same rate and tolerance as the bridge judge's floor). A stolen operator key
   can no longer send a purchase's Bitcoin anywhere but the listing's published address; what it still can do is in the key
   ceremony.
4. **The buyer loop has landed on main** (merged 2026-09-23: `observe`, a performer for every step, the purchase policy, the
   rates and runes indexer, pinned reads). This runbook's cycles budget must be re-read against how often `tick` runs and the
   outcalls each tick makes (every quote step and every signing reads the rate live: six ticker outcalls plus one XRC call).

## What goes on the IC, in order

| Step | What | Who | Reversible? |
| --- | --- | --- | --- |
| 0 | the reproducible build, reproduced by a second person | anyone | — |
| 1 | two canisters created, with cycles: `buyer`, `timelock` | deployer identity (temporary) | yes, until step 6 |
| 2 | the buyer installed, `key_1`, Bitcoin mainnet, 4663 | deployer | yes: reinstall keeps the id and so the keys and addresses |
| 3 | the buyer's addresses read from the IC and written down | anyone | — |
| 4 | the contracts' listing names that EVM address (`docs/deploy/contracts-mainnet.md`, step 4) | Safe | **the vault's buyer never** |
| 5 | the timelock installed: `target` = buyer, `proposer`, `guardian`, `delay_secs` ≥ 72 h | deployer | yes, until step 7 |
| 6 | the handover: the buyer's only controller becomes the timelock | deployer | only through the timelock, 72 h |
| 7 | the timelock blackholed: its own controller list emptied | deployer | **never** |
| 8 | the watcher started | operator | — |

## Step 0: the build, and a second person's build

```
cd autobuyer
scripts/reproducible-build.sh          # Docker, linux/amd64, image pinned by digest; writes autobuyer/build/*.wasm and prints SHA-256s
```

The canonical build is the Docker one: two macOS builds agree with each other but not with Linux (the crate disambiguator
hashes in the name section differ for host-compiled proc-macros; `docs/v2/timelock-controller.md`). A second person, on another
machine, from a fresh clone of the same commit, runs the same command and must print the same three hashes. Record them in
`deployments/canister-ic.json` with the commit. Install `buyer.wasm` and `timelock.wasm` — **never `timelock-rehearsal.wasm`**,
whose floor is 10 seconds, and never gzipped modules (the IC hashes the gzip bytes; `docs/v2/timelock-controller.md`).

## Cycles, and which subnet

Prices from DFINITY's cycle-costs reference ([docs.internetcomputer.org/references/cycle-costs](https://docs.internetcomputer.org/references/cycle-costs/),
read 2026-09-23). 1 trillion cycles (1T) = 1 XDR.

| Cost | Cycles | Note |
| --- | --- | --- |
| create a canister (13-node subnet) | 500B | ×2: buyer and timelock |
| `sign_with_ecdsa` / `sign_with_schnorr`, `key_1` | 26,153,846,153 each | charged at the key's subnet (34-node fiduciary) wherever the caller lives |
| `ecdsa_public_key` / `schnorr_public_key` | 0 | |
| HTTPS outcall, n-node subnet | `(3,000,000 + 60,000·n)·n` + `400·n` per request byte + `800·n` per **reserved** response byte | the doc's version-1 formula; non-replicated calls are priced by use |
| storage (13-node) | 127,000 per GiB per second | negligible here |

What that means for this canister, computed from those prices:

- **One replicated read** (`rpc()` asks every provider; `max_response_bytes = 64,000` reserved per call): on a 13-node subnet
  ≈ 716M cycles per provider, **≈ 1.43B per `rpc()` with two providers**; on the 34-node fiduciary subnet ≈ 1.92B per provider,
  ≈ 3.83B per `rpc()`. The reservation is ~93% of it: a read that reserved 4 KB instead of 64 KB would cost ~8× less. That is a
  code change, noted for the loop's branch.
- **One purchase**: about five `key_1` signatures (announce, pull, the bridge transaction, the Bitcoin spend, settle) ≈ 131B, plus
  its reads — at ~100 `rpc()` calls, ≈ 143B (13-node) or ≈ 383B (34-node). **Roughly 0.3–0.5T a purchase**.
- **The idle loop is the larger bill.** A tick that makes one `rpc()` a minute is ≈ 2.1T a day on 13 nodes (≈ 5.5T on 34). Every
  5 minutes: ≈ 0.4T (≈ 1.1T). The loop's interval is the budget's main dial.

**Subnet.** A 13-node application subnet is cheaper; the 34-node fiduciary subnet (where ckBTC runs) makes every replicated read
agree across 34 nodes rather than 13 — a stronger integrity claim for a custodian, at ~2.7× the outcall cost. The signatures cost
the same either way. This is the operator's choice; `dfx canister create --subnet-type fiduciary` selects it.

**Budget.** Create both with 2T; top the buyer up to **20T** before the handover and the timelock to **2T** before it is blackholed
(anyone can add cycles to a blackholed canister; nobody can then change it). Raise the buyer's freezing threshold to 90 days
(`dfx canister update-settings --freezing-threshold 7776000`) while the deployer still controls it. **Watching the balance is a
public query**, since `canister_status` answers only a canister's controllers: both canisters answer `cycles() -> { balance,
low_threshold }` to anyone (buyer threshold 5T, timelock 0.5T; `docs/v2/trust-points.md` §2), and the watcher alerts once when
either falls below its own published threshold and once when it recovers.

## The install argument (mainnet)

```
(record {
  chain_id = 4663 : nat64;
  vault = "<see: the vault argument>";
  rpc_urls = vec { "https://rpc.mainnet.chain.robinhood.com"; "https://rpc.ordofi.network" };   # docs/v6/phase1-scope.md C8: dRPC stopped serving pinned reads 2026-09-24
  btc_api_urls = vec { "https://mempool.space/api"; "https://blockstream.info/api" };
  key_name = "key_1";
  accept_production_key_name = true;
  bridge_allowlist = vec { <the bridge contracts, checked on 4663> };
  btc_network = "mainnet";   # exactly so: "bitcoin", "Mainnet" and the like are refused at install
  operator = opt principal "<the operator's principal: docs/deploy/key-ceremony.md>";
  wrapper = null;   # the Wrapper's address once contracts step 6 exists; an upgrade sets it ONCE, and it never changes after
  frog_funding = null;   # docs/v6/relay-conversion.md §3.5: null = Relay at 300 bps, no operator float (Phase 1's value)
  # docs/v7/phase1-nft.md §4.1, §9 (decided 2026-09-27): the Ethereum NFT side at launch — D1 Milady, Azuki and MAYC at once, D5
  # holder deposits ON, D7 the default floor rule (null); wrapper and module from the stack file's forCanister.ethNft
  eth_nft = opt record { chain_id = 1; relay_max_fee_bps = 300; deposits = true; floor = null; collections = vec {
    record { collection = "0x5af0d9827e0c53e4799bb226655a1de152a425a5"; wrapper = "<Milady EthNftWrapper>"; module = "<its FloorModule>";
             max_purchase_wei = <≤ custody per-item cap>; max_premium_bps = 500 };
    record { collection = "0xed5af388653567af2f388e6224dc7c4b3241c544"; wrapper = "<Azuki EthNftWrapper>"; module = "<its FloorModule>";
             max_purchase_wei = <…>; max_premium_bps = 500 };
    record { collection = "0x60e4d786628fea6478f785a6d7e704777c86a7c6"; wrapper = "<MAYC EthNftWrapper>"; module = "<its FloorModule>";
             max_purchase_wei = <…>; max_premium_bps = 500 } } };
})
```

- **`btc_api_urls` must be two esplora explorers of the same network that agree.** The local installs used UniSat's API and
  mutinynet, which can never agree on a default-signet transaction (mutinynet is a different signet); the checkpoint check in
  `btcnet.rs` now fails loudly on that. For mainnet: mempool.space and blockstream.info, whose `/block-height/900000` both answer
  the same hash (checked 2026-09-23).
- **`rpc_urls`** must be distinct and must agree (`validate()` refuses duplicates since the rehearsal found one URL listed twice).
- **`key_name = "key_1"`** is the production threshold key; `accept_production_key_name = true` is the installer's statement that
  naming it is deliberate.
- **`frog_funding`** (since 2026-09-26, `docs/v6/relay-conversion.md`): how a floor frog is paid for. `null` means the module's own
  ETH is advanced and converted to BTC at the float through Relay, judged by the order hash, with a 3 % fee bound and **no operator
  float**. `opt record { max_fee_bps = <50..500>; float_first = true }` keeps an optional operator float that is used first when
  it covers the frog. Taken from every upgrade's argument (the watcher decodes and diffs it: a higher fee bound or `float_first`
  turned on is a loosening). An argument encoded before the field existed still decodes (`null`).
- **`operator`** is re-read from every upgrade's argument (the rest comes back from stable memory), so changing it later is a
  timelocked, public proposal.

### The vault argument

`vault` is in `Init` but **no code reads it today** (`grep` finds only its definition). An upgrade keeps the stored value — only
`operator` follows the upgrade argument (`post_upgrade`) — so if the loop starts reading it, it can be set only by the first install
or by a reinstall. The listing that creates the vault needs the canister's EVM address first (contracts runbook, step 4). The
sequence that closes the circle, before the handover:

1. install with a placeholder vault (`0x0000…0001`);
2. read `evm_address()`; list on 4663 with it; read the new vault's address;
3. **reinstall** (`--mode reinstall`) with the real vault — the canister id is unchanged, so its threshold keys, EVM address and
   taproot address are unchanged; read `evm_address()` again and confirm it;
4. then step 5 onward. After step 6 a reinstall is impossible (the timelock only upgrades, never reinstalls).

If the loop serves more than one listing, one `vault` field is not enough: note for the loop's branch.

## The mainnet checkpoint

**Corrected 2026-09-26 (audit D, I-3).** Until then only `broadcast_btc` checked the checkpoint; `btc_tx_seen` and custody's reads
did not. Now `btcnet::ensure_btc_network` shows the checkpoint on every explorer before the custodian's pass or the frog side reads
Bitcoin at all, once per explorer list and at least once a day (and after every upgrade, whose argument may change the list);
`broadcast_btc` still checks on every send.

`broadcast_btc` and `btc_tx_seen` refuse to send or believe anything until every explorer returns a pinned block hash at a fixed
height for the installed network. Mainnet's is added to `checkpoint()` in `src/btcsend.rs` as part of Blocked today, 1:

1. **Who picks it:** the person making the code change proposes a height at least a few months deep (a round height such as
   900,000; the tip was 968,214 on 2026-09-23).
2. **How it is verified — by at least two people, before review is approved:**
   - each runs `bitcoin-cli getblockhash <height>` on **their own** Bitcoin Core node;
   - and reads `https://mempool.space/api/block-height/<height>` and `https://blockstream.info/api/block-height/<height>`;
   - all must print the same hash, and the reviewer writes the values they saw in the review.
3. A candidate, read from both explorers on 2026-09-23 and **not yet checked against a node**: height 900,000 →
   `000000000000000000010538edbfd2d5b809a33dd83f284aeea41c6d0d96968a` (mined 2025-06-06 05:41 UTC).
4. It lands in the wasm, so after go-live changing it is a timelocked upgrade like any other code.

### The published C7 check (solo operator, 2026-09-25)

The operator is solo, so step 2's "two people on their own nodes" cannot be met; `docs/v6/phase1-scope.md` §3.2 accepts, with G2,
public explorers that anyone can re-query in its place. This is that check, as run for the code change (branch `p1-canister`),
read-only HTTPS `GET`s from one machine. **Anyone can repeat every line below** and must get the same hash.

| When (UTC) | Source | Request | Answer |
| --- | --- | --- | --- |
| 2026-09-25 04:36:33 | mempool.space | `https://mempool.space/api/block-height/900000` | `000000000000000000010538edbfd2d5b809a33dd83f284aeea41c6d0d96968a` |
| 2026-09-25 04:36:33 | blockstream.info | `https://blockstream.info/api/block-height/900000` | `000000000000000000010538edbfd2d5b809a33dd83f284aeea41c6d0d96968a` |
| 2026-09-25 04:36:34 | btcscan.org (esplora) | `https://btcscan.org/api/block-height/900000` | `000000000000000000010538edbfd2d5b809a33dd83f284aeea41c6d0d96968a` |
| 2026-09-25 04:36:34 | BlockCypher | `https://api.blockcypher.com/v1/btc/main/blocks/900000` | `"hash": "000000000000000000010538edbfd2d5b809a33dd83f284aeea41c6d0d96968a"`, `"time": "2025-06-06T05:41:39Z"` |
| 2026-09-25 04:36:34 | blockchain.com | `https://blockchain.info/block-height/900000?format=json` | `"hash":"000000000000000000010538edbfd2d5b809a33dd83f284aeea41c6d0d96968a"` |

Five operators, one hash. The tip then was **968,499** on both
mempool.space and blockstream.info (`/api/blocks/tip/height`, 04:36:49), so the checkpoint is 68,499 blocks (≈ 15.7 months) deep.

**The header, checked here rather than believed.** `GET /api/block/<hash>/header` on mempool.space and blockstream.info (04:36:45)
returned the same 80 bytes:

```
00a0ab20247d4d9f582f9750344cdf62c46d81d046be960340960100000000000000000070f96945530651135839d8adc3f40e595118ec74c7ad81a3d17bb022e554fb0c937f4268743702177ad05f92
```

Its double SHA-256, byte-reversed, **is** the hash above; its `bits` (`0x17023774`) give a target the hash is below (the work is real);
its previous block is `0000000000000000000196400396be46d0816dc462df4c3450972f589f4d7d24`, and its time 2025-06-06 05:41:39 UTC,
as BlockCypher and blockchain.com also report. The same computation is a unit test in the canister
(`btcsend::tests::the_mainnet_checkpoint_is_the_double_sha_of_a_header_whose_work_is_real`), so the value in the wasm is tied to
this header, not only to the explorers' word.

What this does **not** give is a check against a node the operator runs. Five public explorers could, in principle, agree on a
wrong block; the header's own proof of work means a forgery costs a block mined at mainnet difficulty, which no test network or
other chain can supply, and serving another network is what the checkpoint exists to catch. Whoever next runs a Bitcoin Core node (the ceremony, or a guardian when one
joins) runs `bitcoin-cli getblockhash 900000` and appends the result here.

## Steps 1–3: create, install, read the addresses

```
dfx identity use <deployer>                      # a temporary controller: see key-ceremony.md
dfx canister create --network ic buyer    --subnet-type <application|fiduciary> --with-cycles 2000000000000
dfx canister create --network ic timelock --subnet-type <same>                  --with-cycles 2000000000000
dfx canister install --network ic buyer --mode install --wasm autobuyer/build/buyer.wasm --argument "$(cat buyer-init.did)"
autobuyer/scripts/reproducible-build.sh verify <buyer id> buyer ic          # must print MATCH
dfx canister call --network ic buyer evm_address                             # the operator or a controller may call it
dfx canister call --network ic buyer taproot_output_key
dfx canister call --network ic buyer buyer_address                           # the buyer's bc1p… and output key
dfx canister call --network ic buyer custody_address                         # the custodian's: a DIFFERENT key and address
```

The two addresses must differ (derivation paths `btc` and `custody`): frogs are held only at the custody address, which no
purchase signs for (`docs/v3/wrapper.md`, "Custody on its own key"). The custody address is the listing's `btcVault` and the
Wrapper's `custodyAddress` (contracts runbook, step 6). Fund it with a float of **plain** sats for delivery fees; the operator
tops it up when `redemptions()` shows a delivery waiting for "no clean confirmed UTXO".

**The custody fee pool** (audit D, H-4; `docs/v6/phase1-scope.md` §3.3). Once the gates allow value in, send custody one
plain-sat output of **at least 20,000 sats** (nothing else in that output) and register it with
`add_custody_fee_input("<txid>:<vout>", <sats>)` as the operator. 20,000 sats is the operator's minimum (2026-09-26), one value in
two places: `script/golive-check.sh --canister <buyer> --min-fee-pool-sats <sats>` (default 20000; FAILs below it) and the
monitor's `min_fee_pool_sats` (default 20000; SEV2 below it). The canister's `MAX_CONFLICT_FEE_SATS` is unchanged at 100,000
sats: a ceiling a re-priced conflict-out may pay, not a requirement, and only an entry that large lets it reach it; an entry
under it is an INFO note from both. In a large fee spike a stuck delivery is cancelled more slowly; the gated Wrapper (refunds only after markUndeliverable or 30 days) keeps that from becoming a loss.

Write `evm_address`, the taproot output key and the address derived from it into `deployments/canister-ic.json` and commit it.
That EVM address is what the contracts runbook's step 4 types into `ETCH_BUYER_CONFIRM`.

## Steps 5–7: the timelock, the handover, the blackhole

```
dfx canister install --network ic timelock --mode install --wasm autobuyer/build/timelock.wasm \
  --argument '(record { target = principal "<buyer>"; roles = record { proposers = vec { principal "<p1>"; principal "<p2>" };
    guardians = vec { principal "<g1>"; principal "<g2>"; principal "<g3>" }; veto_threshold = 2 : nat8 }; delay_secs = 259200 : nat64 })'
autobuyer/scripts/reproducible-build.sh verify <timelock id> timelock ic    # MATCH, against timelock.wasm (72 h floor)
dfx canister call --network ic --identity anonymous timelock status         # target, roles, delay as intended; rehearsal_build = false
dfx canister call --network ic --identity anonymous timelock cycles         # and a buyer `cycles` call, likewise: both answer anyone

dfx canister update-settings --network ic buyer --set-controller <timelock id>      # step 6: the handover
dfx canister info --network ic buyer                                                  # Controllers: <timelock id> — and nobody else

dfx canister update-settings --network ic timelock --freezing-threshold 7776000
dfx canister update-settings --network ic timelock --remove-controller <deployer principal>   # step 7: the blackhole
dfx canister info --network ic timelock                                               # Controllers: (none)
```

The production build refuses `delay_secs` below 72 hours, fewer than 2 proposers or 3 guardians, a veto threshold outside
`2 ≤ k ≤ guardians − 1`, and any principal in both roles (`docs/v2/trust-points.md` §3). Between step 6 and step 7 the deployer
still controls the **timelock** (not the buyer): do step 7 in the same sitting. After step 7 the deployer identity controls nothing
and can be destroyed.

## What anyone can verify afterwards

```
git clone <repo> && cd <repo> && git checkout <commit in deployments/canister-ic.json>
cd autobuyer && scripts/reproducible-build.sh
dfx canister info --network ic <buyer>       # Controllers: exactly the timelock; Module hash: equal to build/buyer.wasm's SHA-256
dfx canister info --network ic <timelock>    # Controllers: none; Module hash: equal to build/timelock.wasm's SHA-256
scripts/reproducible-build.sh verify <buyer> buyer ic && scripts/reproducible-build.sh verify <timelock> timelock ic
dfx canister call --network ic --identity anonymous <timelock> status    # the pending proposal, if any, with its wasm_hash
```

`dfx canister info` asks the IC for its certified record: the controllers and module hash are the IC's word, not the operator's.

## Watching the delay (step 8)

The timelock's delay protects only people who read it. `scripts/timelock-watch.mjs` reads its public `status` and `history` as the
anonymous identity and posts every Proposed, Cancelled (withdrawn), VetoVote, Vetoed, Executed, Failed, Paused, ResumeVote and
RolesChanged event to a Telegram chat, and both canisters' cycles against their published thresholds:

```
TELEGRAM_BOT_TOKEN=<from BotFather> TELEGRAM_CHAT_ID=<the chat> \
node scripts/timelock-watch.mjs --timelock <timelock id> --network ic \
  --expect-target <buyer> --expect-proposers <p1>,<p2> --expect-guardians <g1>,<g2>,<g3> --expect-veto 2 --expect-delay 259200
```

Run it on a machine that is always on (a small VPS, under systemd with `Restart=always`), with `dfx` installed (`DFX=` points at it
if not in the macOS default location). `--dry-run --once` prints instead of posting. It alerts when the timelock's fixed setup is
not the expected one, and says so loudly if a rehearsal build is guarding the IC. `--certified` asks by update calls, which cannot
be forged by a boundary node, at a few million cycles each from the timelock's balance. Tested on the local rehearsal timelock: a
proposal made there was reported, its cancellation was reported, a failed Telegram post was retried, and a fake token never
appeared in any output or in the state file. (That test left two events — Proposed and Cancelled — in that local timelock's
history.) Give the guardian and at least one other person the chat.

## Before real money: rehearse on the IC with the test key

Everything above can be run once on the IC mainnet with `key_name = "test_key_1"` (10B cycles a signature instead of 26.2B), 4663's
testnet (46630) and Bitcoin testnet4 — the only difference from production being the key and the networks. It is the only
rehearsal that exercises real IC outcalls to the real providers (Blocked today, 2, is answered there or nowhere).

## When the other branches land

- **The buyer loop**: re-read "Cycles" (the timer's interval sets the bill), "The vault argument" (single vault vs several), and
  Blocked today 3 and 4. If the loop uses `vault`, the placeholder-then-reinstall sequence becomes mandatory.
- **Bitcoin broadcast** (`src/btcsend.rs`, `src/btcnet.rs`, merged): the explorers and the checkpoint above already follow it.
