# Go-live runbook: the operator's pre-launch checklist, prepared (2026-09-28, branch `golive-prep`)

The checklist is `docs/v7/phase1-nft.md` §5.1 and the pre-launch tasks of `docs/audit/2026-09-27-nft/review7/README.md` and
`review8/README.md`. This file says, item by item, what was done without funds or keys, and what the operator runs, with the exact
commands. **Nothing here was deployed, signed or sent; no cycles or ETH were spent; no key was used.** Every network read made
while preparing it was read-only (JSON-RPC reads, Relay's quote API, OpenSea's and CoinGecko's public stats).

| # | Item | State | Where |
| --- | --- | --- | --- |
| 1 | IC-outcall rehearsal of the canister's Ethereum pair | **Done: paths A and B PASSED on mainnet 2026-10-09** (subnet `4utr6-…-mqe`) | `ops/rehearsal/ic-outcall/` (§1) |
| 2 | A real Relay transfer each way, and a refund (C5-I-1) | **Done 2026-10-09: all four transfers creditable by the canister's judge** (phase1-nft.md §5.1) | `ops/rehearsal/relay/` (§2) |
| 3 | Depth probe of the committed pair | **Done** (baseline below); runs daily from now on | `script/depth_probe.py`, `.github/workflows/depth-probe.yml` (§3) |
| 4 | Monitor configuration | **Done** up to what only the deploy produces | `deployments/launch/monitor-nft.json` (§4) |
| 5 | Runner and service settings, Railway layout | **Done** (templates; no service created). Two image bugs found and fixed | `deployments/launch/railway/` (§5) |
| 6 | Cycles plan and a burn-rate page | **Done** (estimate; the page measures it live) | §6; `monitor/src/watcher.mjs` `burnCheck` |
| 7 | B8-L-1 (no periodic manual probes) | **Fixed** (monitor page + daily workflow; no canister change) | §7; `docs/audit/2026-09-27-nft/fixes-round8.md` |
| 8 | Bitcoin Frogs at launch (the operator, 2026-10-09: ordinals WITH the Ethereum collections) | **Prepared** (launch file, golive-check, monitor, Railway, dry run); review rounds and the funded rehearsal open | §9; `deployments/launch/nft-pilot.json` `bitcoin` |

**`buyer.wasm` CHANGED — a short confirmation review is needed before the ceremony.** The IC rehearsal (§1) found that the buyer's
Ethereum pin and finality reads reserved 16,000 bytes while today's Ethereum blocks (hashes only) are 11–51 kB, and the IC applies the
cap to the raw body: as built, almost no tick could take an Ethereum pin, so no Ethereum read would have run. Commit `173470e` makes
Ethereum block reads reserve 128,000 bytes (`nftnet::block_view_bytes`; 4663 keeps 16,000). That is the only canister source change.
The reproducible build (`docker build --no-cache --platform linux/amd64 -f autobuyer/Dockerfile.build` on a `git archive` of
`173470e`):

```
d38b808d77d807713e3468981d3a16e4d5ae150dcd18607286a156237bdeffc6  buyer.wasm  (7,106,677 bytes; gzip -9 -n 1,351,109)
b23a190365d4c5b2bfc41bd9e9caf147bf2eba50ba3b3d477e2be372840edc41  timelock.wasm            (unchanged)
2145099ea00214e8b521d456f6fc48145c7139f1ecd0c5643393c09c688e62f4  timelock-rehearsal.wasm  (unchanged)
```

Review 8's `b7532c4d…28ec` must not be installed. The monitor and the ops images changed too (§5, §7): see `fixes-round8.md`.

**Superseded 2026-10-08: `buyer.wasm` changed again with the dormant collections (`main` 9e8b84f..07d37c0) and its confirmation review is
`docs/audit/2026-09-27-nft/dormant-fixes-round2-review.md`.** The reproducible build of `07d37c0` (the same command, on its `git archive`):

```
9f0e65ab023ed16cc21c8881ebb3095714563e4fadc4f9d84c07479c3883957f  buyer.wasm  (7,456,262 bytes; gzip -9 -n 1,415,612)
b23a190365d4c5b2bfc41bd9e9caf147bf2eba50ba3b3d477e2be372840edc41  timelock.wasm            (unchanged)
2145099ea00214e8b521d456f6fc48145c7139f1ecd0c5643393c09c688e62f4  timelock-rehearsal.wasm  (unchanged)
```

Neither `d38b808d…effc6` nor `b7532c4d…28ec` is to be installed. Path B below takes this `buyer.wasm`; a second person's build from a
fresh clone of `07d37c0` must print the same three hashes before the ceremony (`canister-mainnet.md`, step 0).

**2026-10-09: no second person** (the operator's decision; `launch-gates.md` G-section reproducible build, waived): the second build is
a fresh clone built on a clean CI runner, and the hashes are published with the release.

---

## 1. The IC-outcall rehearsal (hard precondition, review 7)

The kit is `ops/rehearsal/ic-outcall/` (its README is the full runbook). Two paths, both run by the operator with their own dfx
identity on the LAUNCH subnet, both deleting their canister at the end (a trap) and withdrawing what is left to the identity's cycles
ledger:

- **Path A, the probe canister** (`run.sh`): the buyer's reads made the way the buyer makes them (its caps, its transform names and
  bodies, held byte for byte to the buyer's own functions by `cargo test`): the pin read, block 26,012,500 (hash `0xda4c2b0a…b909`),
  Milady's 10 `Transfer` logs over 26,012,410..26,012,909 (7 moves after the transform) and the receipt of `0x17133e8b…c093`, on
  both providers; consensus, agreement after the transforms, and Tenderly (IPv4 only) reached. Prints PASS/FAIL per check and
  `REHEARSAL: PASS|FAIL`.
- **Path B, the real `buyer.wasm`** (`buyer-throwaway.sh`): the reproducible build in a throwaway canister with a throwaway Init (the
  pair only, nothing custodial, signet, the test key), `nft_chain_status(1)` three times.

```sh
cd ops/rehearsal/ic-outcall
( cd probe && cargo +1.98.1 test )                                   # 7 tests: the transform copies = the buyer's; the verdict
dfx cycles balance --network ic                                      # ~2 T is plenty; most comes back
./run.sh --network ic --subnet <launch subnet id>                    # or --next-to <buyer id> once it exists; type 'rehearse'
./buyer-throwaway.sh --wasm <reproducible buyer.wasm, 9f0e65ab…957f> --network ic --subnet <launch subnet id>
```

**Cycles:** Path A's outcalls ≈ 6.7 B on 13 nodes (≈ 18 B on 34), Path B ≈ 7 B (≈ 20 B); the creation fee is the real cost:
≈ 0.5 T per canister on 13 nodes (≈ 1.3 T on 34). Net ≈ 0.5–0.6 T per path on 13 nodes, ≈ 1.35 T on 34; everything else is
withdrawn on delete (locally: 1.5 T in, 0.902 T back).

**Path A on mainnet: PASS (2026-10-09).** Subnet `4utr6-xo2fz-v7fsb-t3wsg-k7sfl-cj2ba-ghdnd-kcrfo-xavdb-ebean-mqe` (13 nodes, public, 13 node
providers in 12 countries), the operator's identity, probe module hash `0xa0b5…6b6e`, throwaway canister `ilnmx-iiaaa-aaaas-qg6ua-cai`
(deleted). Every check passed on both providers: consensus on the head, the buyer's pin read at 128,000 bytes, block 26,012,500 with its
measured hash, Milady's 10 Transfer logs (7 moves), the receipt of `0x17133e8b…c093`; the pair agreed byte for byte on each; and
**Tenderly's IPv4-only host is reachable from this subnet** (the question only mainnet could answer). The outcalls cost 6.73 B cycles;
the run cost 0.60 T net (1.5 T in, 0.897 T withdrawn on delete). The report: `docs/audit/2026-09-27-nft/ic-outcall-mainnet-2026-10-09.json`.
**Path B on mainnet: PASS (2026-10-09)**, the same subnet: the reproducible `buyer.wasm` `9f0e65ab…957f` in a throwaway canister
(`ifpb7-tyaaa-aaaas-qg6va-cai`, test key, nothing custodial), `nft_chain_status(1)` three times: the buyer's own Ethereum pin read and two
pinned reads, **3 of 3 passed** (pins 26,154,432 and 26,154,440). Its cleanup left the canister standing (dfx's withdrawal wallet could not be
called on the stopped canister, IC0508); started again, the delete withdrew 1.36 T and deleted it. Both scripts now retry that way. Path B
cost ≈ 0.64 T net; one earlier attempt failed before installing (a relative `--wasm` path, fixed) and cost 0.59 T, almost all creation fee.
**The rehearsal is complete: item 1 is done.**

**Verified locally** (dfx 0.32 PocketIC, real HTTPS outcalls from the preparer's machine): the first run FAILED on the pin read at
16,000 bytes (probe and the real old `buyer.wasm`, 3 of 3: `Http body exceeds size limit of 16000 bytes`); after `173470e`, Path A
`REHEARSAL: PASS` (every check, both providers, identical transformed bytes) and Path B's pin read passed on every call that reached
the providers. **Only the mainnet run shows** consensus across the subnet's real replicas, whether the IC's NAT64 reaches Tenderly's
IPv4-only host from the launch subnet, and the exact creation fee. On a Tenderly connect failure: pick another keyless provider that
passes `python3 script/depth_probe.py <url>` and this rehearsal (`run.sh --urls https://a,https://b`), and add it to golive-check's
`DEEP` list.

## 2. A real Relay transfer each way, and a refund (C5-I-1; C7-L-1)

The full runbook is `ops/rehearsal/relay/README.md`. In short (`$ME` is the operator's own public address; nothing here holds a key):

```sh
cd ops
# 1. 4663 -> Ethereum: quote read-only, check "checks: ok", then sign the printed deposit in YOUR wallet on 4663
node --import tsx rehearsal/relay/quote.ts --user $ME --amount-wei 2000000000000000 --dir out
# 2. Ethereum -> 4663: the same, signed on chain 1
node --import tsx rehearsal/relay/quote.ts --user $ME --amount-wei 1000000000000000 --dir back
# 3. a refund each way (Relay's documented forced refund; needs a Relay API key in the environment, never printed)
RELAY_API_KEY=… node --import tsx rehearsal/relay/quote.ts --user $ME --amount-wei 2000000000000000 --dir out --force-refund
# 4. after each: the verdict from the canister's own judge (read-only)
node rehearsal/relay/verify.mjs --us $ME --deposit 0x<deposit tx> --deposit-chain 4663 --payout 0x<fill or refund tx> \
     --payout-chain <1|4663> --request-id 0x<requestId from the quote> --relay-status
```

Record each verdict line in `phase1-nft.md` §5.1 under C5-I-1: a solver transfer naming the order (creditable), a depository
`execute` whose `RelayCallExecuted` pays us (creditable only with the operator's stated amount), or a withdrawal the payee must claim
itself (never creditable: say so in §5.1, as it already warns). The two real depository executes the reviews cite are both
payee self-withdrawals (verified again here). **Relay's keyless `/requests/v2` retires on 2026-11-24**: get a Relay API key for
`/requests/v3` (and for the forced refund) before then. Relay refuses anything under about 0.0001 ETH, so "a few thousand wei" is too
little: 0.002 ETH out and 0.001 ETH back (measured 2026-09-28).

## 3. The depth probe: baseline (2026-09-28T16:08Z, read-only)

`python3 script/depth_probe.py` against the committed pair (`https://rpc.mevblocker.io`, `https://gateway.tenderly.co/public/mainnet`):

```
PASS https://rpc.mevblocker.io: ok (64,139 blocks deep, head 26,077,048; logs 10/10; attempts 1)
PASS https://gateway.tenderly.co: ok (64,140 blocks deep, head 26,077,049; logs 10/10; attempts 1)
PASS the providers' logs of the pinned range are the same (transaction hash and log index)
DEPTH PROBE: PASS (2026-09-28T16:08:26Z; Milady Transfer 26,012,410..26,012,909, receipt 0x17133e8b…c093)
```

The same read through golive-check's own code path (`--phase1 --probe-depth` on an install argument naming the pair):

```
ok   the install argument (--install-arg): the canister's Ethereum provider https://rpc.mevblocker.io passed the depth probe now (the pinned range's 10 Milady Transfer logs, blocks 26,012,410..26,012,909, and its measured receipt; B6-M-1, D7-L-1)
ok   the install argument (--install-arg): the canister's Ethereum provider https://gateway.tenderly.co passed the depth probe now (…)
```

The first log of the range is `0x17133e8b…c093:468` on both. The range is 64,140 blocks deep today and gains 7,200 a day; the probe
asks to re-pin past 400,000 (about 46 days from now, mid-November 2026). **From now on** `.github/workflows/depth-probe.yml` runs
it every day at 06:41 UTC from GitHub's network (no secret), keeps the output 90 days, and on a failure fails the run and opens (or
comments on) an issue labelled `depth-probe`. At go-live, still run `script/golive-check.sh … --phase1 --probe-depth
--install-arg <the argument>` (review 7, task 2): it reads the real argument's providers.

## 4. The monitor configuration

`deployments/launch/monitor-nft.json` holds everything known before the deploy: the five collections in the launch file's order,
their D8 references (re-read 2026-09-28: every `coingecko_id` names its collection's contract on Ethereum, every `opensea_slug`
answers OpenSea's keyless stats; floors 0.925 / 0.654 / 0.843 / 3.18 / 6.25 ETH, OpenSea and CoinGecko agreeing), the staged plan
(`launch_file`), the guardian's pause after 6 h without a good post, the burn-rate thresholds (§6), and the keeper and resolver in
`liveness`. The only `${…}` is `DWELLIR_API_KEY` (a secret, from the monitor's environment). Left as `<…>` is only what the ceremony
and the deploy produce:

| Placeholder | From |
| --- | --- |
| `<timelock canister id>`, `<buyer canister id>`, `<p1> <p2>`, `<g1> <g2> <g3>` | `deployments/canister-ic.json` and the ceremony log |
| `install_arg_hex` | the pre-handover reinstall's argument, from the ceremony log |
| `<… wrapper>`, `<… floorModule>`, `<the canister's EVM address>` | `deployments/stack-4663.json` (the canister's `evm_address()`) |
| `<the guardian EOA>` | `registry.guardian()` |
| `<… host>` | each Railway service's public domain (§5) |

`monitor/test/launch_config.test.mjs` loads the file through the monitor's own loader with stand-ins for those, and fails if a new
placeholder or a secret value appears. Once filled:

```sh
DWELLIR_API_KEY=… node monitor/src/main.mjs --config deployments/launch/monitor-nft.json --once --dry-run   # the loader's verdict
script/golive-check.sh <4663 rpc> deployments/4663.json deployments/stack-4663.json --phase1 --launch deployments/launch/nft-pilot.json \
  --monitor-config deployments/launch/monitor-nft.json --canister <buyer> …                                    # the go/no-go read
node deployments/launch/railway/inline-monitor-config.mjs deployments/launch/monitor-nft.json > /tmp/monitor-config.json
                                                   # its content is MONITOR_CONFIG_JSON on Railway (the image has no deployments/)
```

## 5. Runner and service settings; the Railway layout

`deployments/launch/railway/README.md` is the layout (services, Dockerfiles, start commands, healthchecks, volumes, replicas, who holds
which key, the order at go-live); one `<service>.env.example` per service lists every variable its code reads, with the default, the
launch value and **SECRET** where the operator sets it in Railway (`OPERATOR_IDENTITY_PEM`, `OPENSEA_API_KEY`, `ETH_LOGS_RPC` (keyed
Dwellir URL), `KEEPER_PRIVATE_KEY`, `RESOLVER_PRIVATE_KEY`, `MONITOR_GUARDIAN_KEY`, `DWELLIR_API_KEY`, `ALERT_TELEGRAM_BOT_TOKEN`, …).
Launch values that matter: `RUNNER_PENDING_EVERY_S=120` (the runner refuses over 300), `RUNNER_TICK_EVERY_S=600`, `RUNNER_ETH_NFT=on`,
`ETH_NFT_SLUGS=0x5af0…25a5=milady,…` (the launch file's five, in its order), `ETH_LOGS_MAX_BLOCKS=500`, and for the resolver
`RESOLVER_STATE_DIR=/data` and `RESOLVER_STATUS_PORT=8080` (its defaults are off the volume and no health endpoint).
`node deployments/launch/railway/check-env.mjs` holds each template to the code (all six pass).

**Fixed while preparing this** (`fixes-round8.md`): the runner image did not copy `relay.ts` / `ethnft.ts` and the reconciler image
missed four files; both crashed at start with `ERR_MODULE_NOT_FOUND`. Both now build and start to their configuration refusal, and a
guard test holds every image to what its entry point loads. The runner's keyed `ETH_LOGS_RPC` is scrubbed from its log lines.

## 6. Cycles: burn, starting balance, top-up policy, the page

**Superseded by the dormant model (`docs/v7/dormant-collections.md` §7, 2026-09-29), with its limits auto-scaled from the cycles
balance (§11).** Cycles scale with ACTIVE collections, not listed ones. The allowlist names eleven collections (Identity MD since 2026-10-09), but a DORMANT collection
costs nothing, and how many may be awake at once follows the buyer's own cycles balance, up to the ceiling `max_active` (10). The text
of 2026-09-28 (five collections always on, a 250 T start) is kept below it for the record.

**The auto-scaling (§11), in one line:** `effective_max_active = min(10, ⌊(liquid balance − 10.1 T) / 20.5 T⌋)`, where 10.1 T is the
8 T reserve (the canister's scan floor) plus 7 days of the 0.3 T idle base, and 20.5 T is one awake collection's 7 days at 2.5 T plus
its 3 T exit reserve. A wake starts only if the balance after its 1.5 T still keeps one more awake. Collections with positions are never
put to sleep: they stay awake above the limit, and no new wake starts until a top-up. The figures are `eth_nft.dormancy.scaling` in the
install argument (timelocked).

| Awake | Liquid balance that keeps them | To start the wake that makes it so |
| ---: | ---: | ---: |
| 1 | 30.6 T | 32.1 T |
| 2 | 51.1 T | 52.6 T |
| 3 | 71.6 T | 73.1 T |
| 10 (the ceiling) | 215.1 T | — |

**Prices** (`docs/deploy/canister-mainnet.md`): an HTTPS outcall costs `(3,000,000 + 60,000·n)·n + 400·n` per request byte `+ 800·n` per
**reserved** response byte on an n-node subnet; a `key_1` signature 26.15 B. One provider-read reserving 8 kB is ≈ 0.13 B on 13 nodes
(0.39 B on 34); 32 kB ≈ 0.38 B (1.05 B); 64 kB ≈ 0.72 B (1.92 B). 1 T cycles ≈ 1 XDR ≈ US$1.35.

**The dormant model, estimated** (13-node subnet; outcalls ×≈ 2.7 on 34 nodes; signatures are charged at the key's subnet either way):

| Part | Basis | T a day |
| --- | --- | ---: |
| Idle base (0 ACTIVE) | the 4663 pin, the chain check and the wake desk's `requestCount()` a tick; the runner ticks every 15 min while all sleep (`RUNNER_IDLE_TICK_EVERY_S`), every 10 min otherwise | ≈ 0.1–0.3 |
| Per ACTIVE collection: posts | ≈ 36 a day (the module's 1 h TTL) × (26.15 B signature + ≈ 8 B of home reads) | ≈ 1.2 |
| Per ACTIVE collection: evidence passes | ≈ 36 a day × (≈ 5.5 B listings + ≈ 2.7 B Ethereum pin and finality) | ≈ 0.3 |
| Per ACTIVE collection: the sale scan | each new move's receipt once, log reads every 1–4 h (the back-off) | ≈ 0.1–0.4 |
| Per ACTIVE collection: module views, tickets, the re-dormancy reads | a view every 30 min, a ticket count a tick, 4 checks a day | ≈ 0.2 |
| **Per ACTIVE collection** | | **≈ 1.5–2.2** |
| One wake, once | the 7-day backfill (≈ 101 log reads, ≈ 300–1,500 receipts) and ≈ 10–25 passes of listings | ≈ 0.5–1.5 |

| Scenario | T a day | 34 T at the handover lasts (to zero) | $100 (≈ 74 T) lasts |
| --- | ---: | ---: | ---: |
| Idle, 0 ACTIVE | 0.1–0.3 | ≈ 110+ days | ≈ 250+ days |
| 1 ACTIVE | 1.6–2.5 | ≈ 14–21 days | ≈ 30–45 days |
| 2 ACTIVE | 3.1–4.7 | ≈ 7–11 days | ≈ 16–24 days |
| 3 ACTIVE | 4.6–6.9 | ≈ 5–7 days | ≈ 11–16 days |
| Worst case, bounded | 3 × 10.5 (the scan's day budget) + the base | ≈ 1 day | ≈ 2–3 days |

**The correction.** The earlier table counted about 12 signatures a day for five collections. A floor post is a signature, and an
ACTIVE collection posts about every 40 minutes (the module's one-hour TTL). Always on, the ten allowlisted collections would have
cost about 15–22 T a day.

**Policy (the operator's ≤ $100 budget, 13-node subnet; §11):**

- **34 T in the buyer at the handover** (≈ US$46): the first wake needs 32.1 T liquid, and 1 T is the freezing threshold's share. That
  keeps **one** collection awake for 7 days with the reserve. The rest of the $100 (≈ 40 T, less 2 T for the timelock) stays in the
  operator's cycles ledger for top-ups.
- Not 18–20 T: under 8 T the canister stops its own sale scan (no wake can finish, no buy passes its fresh read), so the reserve starts
  at 8 T, and a collection's exits need their own 3 T (dormant-collections.md §11.5).
- The buyer's freezing threshold stays at 90 days of its idle burn. The scaling reads the **liquid** balance
  (`canister_liquid_cycle_balance128`), so that share is never counted.
- **Top up when the monitor says so** (below), and whenever more collections should be awake than the balance keeps: each 20.5 T keeps
  one more awake (1.5 T more for its wake). The limit rises on the next tick; no upgrade is needed. The command is unchanged:
  `dfx cycles top-up <buyer> <amount> --network ic`.
- Raise the ceilings `eth_nft.dormancy.max_active` (10) or `wakes_per_day` (12), or change the scaling's figures, only by a
  timelocked upgrade; golive-check and the monitor's argument diff word each change (a lower burn estimate, runway or reserve is a
  LOOSENS).
- `golive-check --launch` prints the formula, the balance each awake count needs, and at the start `effective_max_active` and the wake
  headroom. It FAILs unless `cycles.startT − cycles.freezeT` gives `effective_max_active ≥ 1` and lets the first wake start.

**What to expect.** After the first wake the 34 T start leaves about 32.5 T, so `effective_max_active` falls to 0 within about a day of
one collection's burn: from then no new wake starts until a top-up, and if the awake collection holds positions the monitor pages
`eth-capacity-short`. That is the design: capacity follows funding.

**Automatic top-ups (2026-10-09):** the fueler (`docs/v7/cycles-from-fees.md` §4.2.1) tops the buyer up from the platform's ops share of
the fees when it is under 40 T, at most 20 T a run and 60 T in 7 days. It is built and tested, not deployed: until it runs in `send`
mode, the top-ups below are by hand, and they stay the fallback after.

**The pages:**

- **`eth-capacity-short`** (SEV2, persistent; the Ethereum NFT checker, on the canister's `dorm.cap`): `effective_max_active` is below
  the number of collections with positions. Top up; the page names the balance at which the next wake starts.
- `/status` → `eth_capacity`: `effective_max_active`, the ceiling, the awake and pinned counts, the wake headroom and the day's wakes
  left. The runner's `GET /capacity` and the site's Launch page ("N of M collections can be awake right now") show the same.
- The burn page (`cycles`): SEV2 under 7 days at the measured rate, SEV1 under 2; the rate over `burn_warn_t_per_day` 10 (about 3.5
  times one awake collection; raise it to about 1.5 × (0.3 + 2.5 × `effective_max_active`) T as top-ups raise the limit); and the
  balance under `min_balance_t` **12** (the 8 T reserve plus one collection's exit reserve).

---

*The text of 2026-09-28, five collections always on (superseded):*

**Normal burn, five collections, estimated** (2026-09-28) (13-node subnet; ×2.7 on a 34-node one):

| Part | Basis | T a day |
| --- | --- | ---: |
| Buy pass, module reads | 11 reads × 2 providers × 5 collections × 144 ticks (600 s), 8 kB | ≈ 2.1 |
| Floor evidence | ≈ 5–6 B a collection-pass (listings + new receipts, `fixes-round1.md`), 72 passes (20 min) | ≈ 2.0 |
| Sale scan | ≈ 0.2–0.4 T a collection (`fixes-round2.md`) | ≈ 1–2 |
| Pins and the rest of a tick (4663 line, arrivals, deliveries) | ≈ 10 replicated reads a tick at 64 kB | ≈ 2 |
| Ethereum pin and finality at 128 kB (`173470e`) | ≈ 1.33 B a provider-read; a few a tick, more while a transaction is out | ≈ 1–2 |
| Signatures | ≈ 12 a day (mints, posts, deliveries) | ≈ 0.3 |
| **Total** | | **≈ 6–10** (≈ 16–27 on 34 nodes) |

**Worst case** (documented, `fixes-round5.md` §3): the sale scan is bounded at ≈ 10.5 T a day a collection, **≈ 52 T a day for five**
on 13 nodes (≈ 140 T on 34), whatever the runner does; with the rest of the tick ≈ 60 T a day. (Round 4 also bounded an evidence flood
at ≈ 9 T a day a collection, not re-measured since; were both hit on every collection at once, ≈ 100 T a day.) Under 8 T the canister
stops its own sale scan (the floor is still posted from the sales already read); frozen, it cannot deliver.

These are estimates from the code's reservations. **The live number** is the monitor's: `/status` → `cycles` shows each canister's
balance, its burn a day over the last 24 h (a top-up never counted against it) and the days left, from 6 h after start.

**Policy (13-node subnet; on 34 nodes multiply the amounts by 2.5):**

- **Starting balance: 250 T in the buyer at the handover** (≈ 25–40 days of normal, ≈ 4 days of the worst case), and the timelock at
  2 T as `canister-mainnet.md` says. Set the buyer's freezing threshold to 90 days before the handover (`dfx canister update-settings
  --freezing-threshold 7776000`), while the deployer still controls it.
- **Top up to 250 T whenever the buyer is under 100 T**, and at once on any cycles page. Anyone can top up a canister (no controller
  needed; the buyer is the timelock's after the handover): `dfx cycles top-up <buyer> 150000000000000 --network ic` (150 T) from the operator's cycles
  ledger balance, or `dfx canister deposit-cycles`. **Who:** the operator; the guardians are the backup (each should know the command).
  Keep ≥ 300 T (≈ US$400) ready in the operator's cycles ledger, i.e. four days of the worst case.
- **Look weekly** at `/status` → `cycles`. After the first week, set `cycles.burn_warn_t_per_day` to about three times the measured
  normal (the configured 30 T assumes ≈ 6–10 T normal).

**The page** (`monitor/src/watcher.mjs` `burnCheck`, thresholds in the configuration's `cycles`): SEV2 when the buyer or the timelock
has under 7 days left at the measured rate, SEV1 (persistent) under 2 days; for the buyer also SEV2 when it burns over 30 T a day and
when it holds under 20 T. These come before the older level pages (the buyer's own 5 T threshold, G8's 2 T), which at the worst case
fire only hours before the canister freezes. Tests: `monitor/test/cycles_burn.test.mjs` (a normal 8 T a day pages nothing; the worst
case pages the rate at 6 h, under 7 days at once and SEV1 at 1.85 days; a top-up is not counted against the rate).

## 7. B8-L-1: no periodic manual probes

Fixed on the monitor's side, as review 8's second option (details and the tests: `docs/audit/2026-09-27-nft/fixes-round8.md`). The
monitor reads `SaleScan.down` and pages **`eth-scan-window-unread`** (SEV2, persistent) when the walk back to the window's start has
not moved for 12 h while it is still 600 blocks or more above the start: one provider of the pair that lost its log retention is never
taken as "too deep" by the canister (B7-L-1), and this is its trace. The daily depth-probe workflow (§3) is the earlier,
independent watch. **Review 8's extra task ("watch `down` by hand, or re-run `--probe-depth` periodically") is no longer needed.**

When `eth-scan-window-unread` or the workflow fires: run `python3 script/depth_probe.py` (both providers); a failing provider is
replaced by a timelocked upgrade naming another keyless provider that passes this probe AND the IC rehearsal (§1); add it to
golive-check's measured list (`DEEP`) with the date.

## 8. Test bar (this branch)

Run on `golive-prep` (2026-09-28):

| Suite | Result |
| --- | --- |
| Canister `cargo test` (1.98.1), after `173470e` | **716 passed**, 0 failed (713 at review 8, + 2 B8-L-1 regressions, + 1 block-cap test) |
| IC probe `cargo test` (`ops/rehearsal/ic-outcall/probe`) | 7 passed |
| monitor | **238 passed** (219 before: + audit_nft_d_r8, cycles_burn, launch_config) |
| ops | **68 passed**; typecheck clean (+ the image-imports guard and the ETH_LOGS_RPC scrub) |
| indexer / keeper / tools/snapshot | 170 / 150 / 19 passed |
| `script/test_depth_probe.py` | 10 passed (offline fakes) |
| `deployments/launch/railway/check-env.mjs` | all six templates ok |
| `scripts/golive-dryrun.sh stack` | **DRY RUN: ALL PASS** (146 / 536 / 664 checks), with golive-check's probe now imported from `script/depth_probe.py` |
| Images | runner, reconciler and monitor built; runner and reconciler start to their configuration refusal (they crashed with `ERR_MODULE_NOT_FOUND` before) |
| Reproducible build of `173470e` (`--no-cache`, linux/amd64) | `buyer.wasm` **`d38b808d…effc6`**; timelock and timelock-rehearsal unchanged |
| Local IC rehearsal (PocketIC, real outcalls) | Path A `REHEARSAL: PASS` after the fix; Path B pin read passes |

Not run here: the forge suites (no contract changed: `git diff 47c6cf5 -- src test` is empty).

## 9. The Bitcoin collection at launch (Bitcoin Frogs)

**The operator's decision (2026-10-09): ordinals launch WITH the Ethereum collections**, not as the second wave
`docs/v8/ordinals-wave.md` §3.5 recommended. **Bitcoin Frogs** (`bitcoin-frogs`, the collection the Bitcoin side was built for) is the
canister's one Bitcoin wrapper. The launch file names it in its `bitcoin` block (`deployments/launch/nft-pilot.json`, the reasoning in
its `_bitcoin` note); `script/golive-check.sh --launch` reads it. Nothing here was deployed, signed or sent.

### 9.1 Custody: the caps at each step (A4-L-1)

The Bitcoin wrapper keeps stage 0's **3 items / 10 ETH** (H2, `launch-gates.md` G7; `custody_caps` key `btc:0x<wrapper>`) at every
step. Every item in custody must be able to leave within the timelock's 72 h less a day: two usable days at `global_items` a day, so
**48 items in all at the install's 24 a day** (each collection alone: at most 12 × 2 = 24). The Ethereum items at steps 1 and 2 drop
by 3, taken from **Azuki (17 → 14)**, whose per-item headroom is the deepest (J7's 22 at its 0.67 ETH floor; 14 items are 9.4 ETH of
its 15 ETH):

| Step | Milady | Azuki | MAYC | Pudgy | BAYC | Doodles, Moonbirds, CloneX, Lil Pudgys, Checks | Identity MD | **Frogs** | Global items (binding) | `global_items` a day | Can leave in 2 days |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 install | 13 | **14** | 13 | 3 | 2 | 12 each | 8 | **3** | **48** = 45 + 3 | 24 | 48 |
| 2 outflow raise | 13 | 14 | 13 | 3 | 2 | 12 each | 8 | 3 | 48 | 31 | 62 |
| 3 caps back to J7 | 16 | 22 | 17 | 4 | 2 | 24 each | 8 | 3 | **62** | 31 | 62 |

Step 1: the pilot five 13 + 14 + 13 + 3 + 2 = 45, plus Frogs' 3 = 48 = the global = 24 × 2. Step 2 changes `global_items` alone (24 →
31). Step 3 changes the item caps alone (`global_items` stays 31): J7's pilot five are 61, plus Frogs' 3 = 64, which would need 32 a
day; so step 3's global is **62** = 31 × 2, the most that leaves in two days, Azuki back to 22, Frogs still 3. (With the dormant five's
12 each the global already binds, and the canister's rule is on min(sum, global), so 48 held before the shave too; the shave keeps the
pilot five plus Frogs inside the global, as option (c) had the pilot five alone.) **Identity MD** (the operator, 2026-10-09; dormant)
holds 8 at every step (J7's min(24, 15 / 1.78)) and moves no global: the sums become 116 at steps 1–2 and 192 at step 3, still over
48 and 62, so min(sum, global) is unchanged (48 ≤ 2 × 24, 48 ≤ 2 × 31, 62 ≤ 2 × 31) and its own 8 ≤ 2 × 12. Its D8 reference is
OpenSea alone (`nft-pilot.json` `priceRefs` / `_priceRefs`; monitor `price_refs: ["opensea"]`): the guardian pauses its poster at more
than 20 % from OpenSea, and a missing OpenSea answer pages but never pauses; every other collection still needs both references. golive-check's staged plan and its exit rule count
Frogs' cap in every step (`btc:0x<wrapper>`), and `--proposal --stage 3` holds a step-3 argument to the 62.

### 9.2 Deploy order: one mixed stack

The canister is installed first (`wrapper = null`); read its `evm_address()`, `custody_address()`, `float_address()` and
`buyer_address()` (operator calls; all distinct).

1. `script/deploy.sh` and the Safe's `wire` / `proposeRoles`, as before.
2. **The Bitcoin listing** (`docs/deploy/contracts-mainnet.md` step 4), with the membership root of
   `deployments/launch/btc/bitcoin-frogs-members.json` (10,000 ids, two sources agreeing: `deployments/launch/btc/README.md`):

   ```sh
   ETCH_REGISTRY=<registry> ETCH_LIST_KIND=inscriptions ETCH_LIST_NAME="Bitcoin Frogs" ETCH_LIST_REF=bitcoin-frogs \
   ETCH_LIST_ROOT=0x801f71bbb4c7ee37759a3b23da76bc3dd5ec132c0dd10edefe3dc86acfae418e \
   ETCH_LIST_BTC_VAULT=<custody_address()> ETCH_BUYER=<evm_address()> ETCH_BUYER_CONFIRM=buyer-is:4663:<evm_address(), lowercase> \
   ETCH_MAX_IN_FLIGHT=<the minimum> script/list-auto.sh https://rpc.mainnet.chain.robinhood.com
   ```

   On 4663 it sends nothing and prints the Safe's `listInscriptionsAuto` calldata; the Safe sends it. The buyer is immutable in the
   vault; the root and the BTC vault change only by `propose…` + 7 days.
3. **One `script/deploy-stack.sh` run, the default kind** (no `ETCH_STACK_KIND`): the Bitcoin collection AND the eleven Ethereum
   collections, one stack file (`deployments/stack-4663.json`), so nothing merges stack files:

   ```sh
   ETCH_STACK_CONFIRM=deploy-etch-stack:4663:<deployer> ETCH_REGISTRY=<registry> ETCH_WRAPPER_LISTING=<the listing of step 2> \
   ETCH_STACK_SLUG=bitcoin-frogs ETCH_WRAPPER_NAME="Wrapped Bitcoin Frog" ETCH_WRAPPER_SYMBOL=wFROG \
   ETCH_WRAPPER_CUSTODIAN=<evm_address()> ETCH_WRAPPER_CUSTODY_ADDRESS=<custody_address()> ETCH_STAGE=0 ETCH_CAP_CEILING=<wei> \
   ETCH_OPS=<ops> ETCH_GUARDIAN=<guardian EOA> ETCH_WAKE_BOND=2000000000000000 ETCH_WAKE_WINDOW=604800 ETCH_NFT_CHAIN_ID=1 \
   ETCH_NFT_COLLECTIONS=… ETCH_NFT_NAMES=… ETCH_NFT_SYMBOLS=… ETCH_NFT_SLUGS=… ETCH_NFT_NORMAL_RAISES=… ETCH_NFT_EARLY_RAISES=… \
   ETCH_NFT_CONV_BPS_LIST=… ETCH_NFT_RESERVE_BPS_LIST=…   (the launch file's lists, in its order, as for the NFT stack) \
   script/deploy-stack.sh https://rpc.mainnet.chain.robinhood.com <deployer> --ledger
   ```

   The labels are the launch file's (`bitcoin.slug`, `.name`, `.symbol`): golive-check holds them, and `KEEPER_OW_SLUGS` keys on the
   slug. The Ethereum collections' vaults read the pause flag of `ETCH_NFT_LISTING`, which defaults to this listing: one pause then
   stops both sides' mints. Name another listing there if the operator wants the sides paused apart (a decision; nothing requires it).
   `ETCH_CAP_CEILING` is one value for every vault of the run.
4. The Safe's owner steps DeployStack prints, the Bitcoin ones first: `WrapClaim.initMintLimit(3)` and
   `WrapClaim.wireFloorModule(<frogs floorModule>)` **before any mint**; then `setOps`, `addCollection` and `directory.add` for every
   collection, `setGuardian`, `proposeCapCeiling` (`docs/v6/phase1-scope.md` §3 step 7).
5. `script/golive-check.sh … --phase1 --launch deployments/launch/nft-pilot.json --before-reinstall …`: it now also holds the Bitcoin
   collection's slug, name and symbol, the listing's root to `bitcoin.membersRoot`, the members file, and the staged plan and exit
   rule with Frogs' 3 counted.

### 9.3 Before the handover: the controller-only setters

After the handover the timelock can only Upgrade / Controllers / Roles; it cannot call these. Make them, in this order, with the
deployer (controller) identity, **after the pre-handover reinstall and before the handover** (each refuses while a tick runs):

1. **The reinstall** names the Bitcoin side in its `Init`: `wrapper = opt "<frogs wrapper>"` (set once from nothing),
   `wrapper_from_block = opt <the stack file's fromBlock>`, `frog_funding = opt record { max_fee_bps = 400; float_first = false }`
   (`bitcoin.frog_funding`; null would be 300 bps), the custody cap `btc:<frogs wrapper, lowercase>` at 3 items / 10 ETH / one item up to
   10 ETH beside the Ethereum ones (global 48 items, 75 ETH), two explorers in `btc_api_urls`, `holder_deposits` null (Bitcoin holder
   deposits stay off).
2. `dfx canister --network ic call <buyer> set_floor_sink '("<frogs floorModule>", null, null, null)'` (floornet.rs: a Bitcoin
   sink names no chain, collection or NFT wrapper; added once, never changed).
3. `dfx canister --network ic call <buyer> set_item_config '(opt "<frogs factory>", null, vec {})'` (frognet.rs: the factory set
   once; **no desk**, phase1-scope §1; no extra queue).
4. `dfx canister --network ic call <buyer> set_purchase_limit '("<frogs floorModule>", 500_000_000_000_000_000 : nat)'` (0.5 ETH,
   `bitcoin.maxPurchaseWei`: the policy's `MAX_PURCHASE_WEI`, pinned; it needs the sink of step 2 first).
5. `dfx canister --network ic call <buyer> set_ow_creators '(vec { … })'` (protected-listing review P-01, `3228c5f`): the only
   addresses a protected Ordinals Wallet sale may pay a royalty to. **Left empty, every royalty-paying protected sale is refused**
   (the rest still buy). Pin Bitcoin Frogs' creator address(es) here only if the operator wants those listings; the listing's own
   `creator_address` is never believed. Controller-only: after the handover only an upgrade can change it.
6. **Not** `set_offer_board`, `set_sale_policy` or a desk (phase1-scope §1: hand-ins, YOYO and the desk stay off).

Then, with the **operator** identity (these stay callable after the handover):

7. `set_unisat_key '("<UniSat key>")'` — **after the install and after EVERY upgrade** (the key is lost at an upgrade, never
   echoed); then a keyed `probe_frog` on a known frog (its shapes are unexercised: `ordinals-apis.md` §6.1). Without it contents
   verify on one per-output provider and every deposit, delivery and buy waits.
8. **The custody fee pool seed:** one plain-sat output of at least **20,000 sats** (`bitcoin.custodyFeePoolMinSats`) to
   `custody_address()`, then `add_custody_fee_input '("<txid>:<vout>", <sats>)'`. The monitor's `expect_inbound` announces it.
9. **The float's first padding seed:** about **1,200 sats** to `float_address()` before the first conversion (ordinals-wave R10: the
   padded listing shape needs two float inputs before the seller's). The monitor announces it too.
10. `script/golive-check.sh … --phase1 --launch deployments/launch/nft-pilot.json --monitor-config <the filled monitor file>
   --canister <buyer> --canister-evm <evm_address()> --install-arg <the reinstall's argument>`. For the Bitcoin side it FAILs unless:
   the canister names the cap `btc:<frogs wrapper>` at the launch file's 3 items / 10 ETH (the install's `wrapper` set);
   `floor_state()` holds a Bitcoin floor sink for the frogs module and its limit is 0.5 ETH (a missing limit is a note while the
   launch file's is the policy's default); `ord_providers()` names `unisat`; the fee pool holds one usable entry of at least 20,000
   sats; the install argument names this wrapper and `frog_funding` 400 / false; the monitor's Bitcoin canary watches the wrapper's
   custody address, the float and the buyer on two esplora hosts.

The services (`deployments/launch/railway/`): the keeper posts the Bitcoin module's floor from the Ordinals Wallet book
(`KEEPER_OW_SLUGS=bitcoin-frogs=bitcoin-frogs`, `KEEPER_MODE=send`); the runner converts frog advances through Relay
(`RUNNER_RELAY=on`); `RUNNER_OW_SLUG` / `RUNNER_OW_MEMBERS` stay unset until the protected-buy build is reviewed and a funded rehearsal
has passed, so **Bitcoin buys are the operator's manual hand-ins** (`npm run offer`) until then; the reconciler and the monitor read
the Bitcoin module from the one stack file; the monitor's `bitcoin` block is filled from the three canister addresses.

### 9.4 Open items (not done here)

- **Review rounds: done, 2026-10-09.** The Bitcoin conversion path (C-01..C-08) and the Ordinals Wallet protected-listing code
  (P-01..P-09) were each reviewed; every finding is fixed with its regression test (`3228c5f`, `eedd47e`;
  `docs/audit/2026-10-09-conv-review-fixes.md`; canister `cargo test` 838 passed). That build has a **new `buyer.wasm` hash**: the
  reproducible build, its clean-runner second build (no second person: §0 note) and the Ethereum go/no-go are repeated on it (ordinals-wave §3.5), and the fixes
  themselves get a confirmation read before install.
- **The funded rehearsal** (one real Frog bought end to end, about 375,000 sats: Ordinals Wallet's submit path with the canister's
  signatures, a venue PSBT judged, UniSat's `create_bid` shape) — **declined by the operator** for now. Until it passes, no automatic
  Bitcoin buying (`RUNNER_OW_SLUG`, `RUNNER_UNISAT_COLLECTION` unset).
- Still required before value enters custody (phase1-scope step 11): the TRUC signet delivery rehearsal, and both explorers
  accepting version-3 transactions; UniSat's keyed shapes confirmed (the keyed `probe_frog` above).
- **The mixed dry run** (`scripts/golive-dryrun.sh stack`, phase `rehearse_mixed_stack`): the Bitcoin listing with the launch file's
  root, then ONE `deploy-stack.sh` run of the default kind with the (then ten, now eleven) Ethereum collections, read whole by golive-check `--launch
  --canister --install-arg --monitor-config` (stand-in dfx), and each Bitcoin setter left undone refused. Run 2026-10-09 with
  `STACK_FORK_RPC=https://rpc.mainnet.chain.robinhood.com STACK_FORK_BACK=20 STACK_PHASES=mixed DRYRUN_NO_DRAND=1`: **DRY RUN: ALL
  PASS** (1,389 checks ok in the mixed read). The archive RPC `rpc.ordofi.network` (the default) answered "historical state … is not
  available" from most of its backends at every depth that day, and the public RPC keeps about 15 minutes of state, so the full run
  (`STACK_PHASES` unset) outlives it: the `phase1` phase passed alone (ALL PASS), the `nft` phase passed every go/no-go read and then
  anvil crashed on the public RPC's history window at the floor posts. Re-run the full stack on a working archive RPC before the
  ceremony.

## 10. Rollback

- **No frog in `Handed` before rolling back to a pre-v8 build** (protected-listing review P-07, 2026-10-09). A pre-v8 build does
  not know the phase or its `ProtectedRec`: a handed sale could neither be watched nor abandoned. Check `frog_buys` first; wait for
  every handed frog to reach `Sent`/`Held`, or to be abandoned, before the rollback proposal.
