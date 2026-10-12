# Staging: trying etched end to end before anything is real

## It is hosted, and these are the addresses

| | |
|---|---|
| The chain | https://staging-chain-production.up.railway.app — `/healthz`, `/info`, `/faucet`, JSON-RPC at `/` |
| The API | https://staging-api-production-1b4b.up.railway.app — `/v1/health`, `/v1/coins`, … |
| The site | https://etched.meme/?net=staging#/staging — the app routes on the hash, so `#/staging` is the page and `?net=staging` is what puts every page on the staging chain |

Brought up 2026-09-21 on Railway project `etch`, services `staging-chain` and `staging-api`, each with its own volume; the
smoke test's twelve checks passed against these addresses over the public internet, not against a machine's localhost.
Chain id 9466301, forked from Robinhood Chain at block 69014767. Play money; no key of any value exists on it.

Two tiers. **Tier 1** is ours, hosted, free and safe: a private copy of Robinhood Chain where every contract and every screen can
be used with play money. **Tier 2** is a small real test on Robinhood Chain itself, which only the owner can run, by hand, with
their own keys. Tier 2 has been **prepared and never run**.

## Tier 1: the hosted fork (chain id 9466301)

### What it is

One service runs `anvil` forking Robinhood Chain mainnet at a pinned block, reporting chain id **9466301**, a number no real network uses. Wallets show it as *etched staging*, and its money as
**sETH** ("staging ETH"), so that it cannot be mistaken for money. Because it is a fork,
the real Pons factory, curves and fee machinery are there, byte for byte. Our contracts are deployed onto it at first boot by the
same guarded wrapper used everywhere else. State is saved to a volume, so coins and draws survive a restart.

Nothing on it has value. **No private key exists anywhere in staging**: the roles (owner, keeper, purchaser, attestor, treasury)
are random addresses nobody holds a key for, and the node impersonates them for calls that come from inside the container only.

### What a stranger can and cannot do

The node listens on 127.0.0.1. The only public door is a proxy that passes an **allowlist** of read methods plus
`eth_sendRawTransaction` (a transaction you signed yourself). Everything else is refused: every `anvil_*`, `evm_*`, `hardhat_*`,
`debug_*`, `admin_*`, `personal_*` method, and `eth_sendTransaction`, `eth_accounts` and `eth_sign*`, which on an impersonating
node would let anyone act as the owner. Log queries are bounded to 500,000 blocks. Requests are limited per address.

| Path | What |
|---|---|
| `POST /` | JSON-RPC, filtered |
| `POST /faucet` `{"address":"0x…"}` | play ETH, once a day per address, capped per IP and in total; refuses the deployment's own addresses |
| `GET /healthz` | node up, chain id, block, whether the bootstrap finished |
| `GET /info` | the deployment (addresses, fork block), the shortened waits, and the word "staging" |
| `GET /snapshots/<file>` | the snapshot files the operator published, so anyone can rebuild a draw's root |

### What is real and what is simulated

| Real | Simulated |
|---|---|
| Pons (forked): launch, curve, fees, harvest | The ETH (faucet money) |
| Our contracts, compiled from this repository, unchanged | **Bitcoin prices**: a fixture, not a marketplace |
| The 80/20 split, caps, declare, release, settle, draws, claims | **The Bitcoin purchase**: nobody buys anything; the receipt is a fixture txid |
| **drand**: real rounds from the real beacon, verified on chain | **Delivery**: a fixture txid is recorded |
| The snapshot tool and the root it builds | The role holders: one operator process plays keeper, purchaser and attestor |
| The indexer and the website | The long waits (below) |
| The clock: block time is the real time, brought to the present at every boot | **State nobody has touched yet**: read from Robinhood Chain as it is *now*, not as it was at the fork block (below) |

### A fork that outlives its block

The public RPC serves historical state for about fifteen minutes. After that, a fork pinned at its block cannot read anything it
has not already cached, which would stop the faucet and every new wallet. So the node reads the real chain through a small
read-only shim on 127.0.0.1 (`staging/chain/upstream.ts`): when a state read is refused as too old, it is asked again at the
present head. A wallet that has never been used is empty at both moments; Pons's contracts are read mostly in the first minutes and
then cached for good. In principle a contract could be seen partly at the fork block and partly now. `/healthz` reports how many
reads fell back (`upstreamFallbacks`). Pointing `STAGING_UPSTREAM` at an archive node removes the question.

### The waits, and how they are shortened without touching the contracts

The contracts hard-code their waits (`DECLARE_DELAY` 1 h, `CHALLENGE` 15 min, `DESTINATION_CORRECTION` 1 h). They are
`constant`, there is no test switch, and **we did not add one**: a switch that shortens a safety wait is a thing that can be
left on. Time travel was also rejected, because a node whose clock runs ahead cannot verify real drand rounds.

Instead the operator, from inside the container, rewrites **one stored timestamp per wait** on the fork (`anvil_setStorageAt`):
a purchase's `releasableAt`, a lot's or distribution's `challengeEnds`, a lot's `destinationAt`. Each is moved so that
`STAGING_WAIT_S` seconds (default 120) remain. The code that checks the wait is the mainnet code. This is possible only on a
node that offers `anvil_*` on localhost; it is impossible on a real chain. `/info` lists what is shortened.

Deadlines measured in days (settle 3 d, delivery 7 d, claim 30 d, redraw 24 h, void 7 d) are **not** shortened: nothing in a
happy path waits for them. The 7-day owner timelocks are not shortened either.

### Using it

1. Open the staging site (the ribbon says STAGING). On **Staging**, add the network to your wallet and take faucet ETH.
2. Launch a coin, buy and sell it with the staging-only trade box on the coin's page.
3. Fees are harvested and split by the operator every tick. When a coin's vault can afford the fixture price the operator posts a
   cap, declares, waits the shortened wait, releases to itself and settles with a fixture receipt.
4. The creator chooses what happens to the asset on **Draws**. If they do not, the operator chooses a weighted draw as the
   creator after `STAGING_CHOOSE_AFTER_S`.
5. The operator builds the snapshot, opens the draw, pins a real drand round, resolves it. The winner posts a Bitcoin address;
   the operator records a fixture delivery.

To settle with a **real** Bitcoin txid instead of the fixture (for instance one you made on Bitcoin testnet or mainnet yourself),
set `STAGING_MANUAL_SETTLE=1` and run inside the container: `SATS_PAID=<sats> npm run attest -- <listing> <purchase> <txid> member:<N>` for an inscription (N is a fixture member of
the staging collection, 0 to 1023, that nobody holds yet: the txid is what is real, the inscription id is not), or
`… <txid> <rune id> <raw amount>` for a rune.

### Why a fork of mainnet, and not Robinhood's public testnet

Robinhood runs a public testnet: `https://rpc.testnet.chain.robinhood.com` answered `eth_chainId` with `0xb626` (46630) on
2026-09-21, with a head past block 122 million. We do not use it, because **Pons is not there**: the factory and router addresses
we use on 4663 have no code on that testnet (`cast code` returns nothing; on mainnet they are 24 kB and 12 kB). Everything etched
does starts with a Pons launch and Pons's fee machinery, so a chain without Pons can test nothing that matters. A fork of mainnet
carries the real Pons, byte for byte. Our fork's chain id was first 46630 by coincidence of the brief; it was changed to
9466301 because a wallet that knows the public testnet would mis-route "add network", and because sharing an id with any real
network defeats the point of having our own. Every guard now refuses 4663 **and** 46630 by name. The id lives in one file,
`staging/chain-id.json`: the staging package, the keeper's guard and the deploy wrapper read it; the contracts' script, the
indexer and the website each hold one constant that a test compares with it.

### Running it locally

```sh
cd tools/snapshot && npm ci && cd ../../keeper && npm ci && cd ../staging && npm ci
STAGING_DATA=/tmp/etched-staging PORT=8650 npm start      # node + bootstrap + proxy + operator
# another shell: the indexer against it
cd indexer && RPC_URL=http://127.0.0.1:8650 DEPLOYMENT_FROM_RPC=1 DB_PATH=/tmp/etched-staging/etch.db PORT=8797 npm start
# another shell: the smoke test (12 checks, about five minutes, needs the drand beacon)
cd staging && npm run smoke -- http://127.0.0.1:8650 http://127.0.0.1:8797
# the site
cd web && VITE_ETCH_STAGING_RPC=http://127.0.0.1:8650 VITE_ETCH_STAGING_API=http://127.0.0.1:8797 npm run dev
```

Hosting it: `staging/RAILWAY.md`. Resetting it: stop the service, delete everything in `/data`, start it.

### What you are looking at: all of it is simulated

Staging runs a set of simulators (docs/phase8.md), because a page full of nothing tells you nothing about whether the page is any
good. **Every coin there was launched by the simulator, every trade was made by it, and every price, Bitcoin receipt, delivery
and payout is invented.** Nothing on Bitcoin exists: no inscription, no rune, no transaction, no wallet. Receipts are recorded as
`staging-fixture`, and the pages label them.

What the simulator does, so that every page has something on it:

- **Coins.** Up to fifteen, launched a few minutes apart, with creator taxes across the whole 0–10% range, generated logos, and
  names that say what they are — *Basalt (sim)*, `BASASIM`.
- **Trading.** Constant, on Pons's real curve, with moods: steady coins, momentum bursts, faders, and *runners* that buy until the
  curve sells out and Pons graduates them into a pool. One coin is sniped in its first seconds so the anti-snipe tax shows.
- **Harvests.** A wallet with no connection to the coin calls `harvest()` on a schedule — it is permissionless — so vaults fill
  again and again, and the 80/20 split is visible repeatedly rather than once.
- **Every state at once.** One declaration is always counting down, one release is always awaiting its receipt, one purchase has
  been returned unspent, one written off, one draw sits inside its challenge window, one is pinned waiting for its drand round,
  one waits on a winner's Bitcoin address, one delivery is overdue. Without this the pages would only ever show the states a
  single run happens to pass through.
- **Winners' addresses** are BIP-350 test vectors: valid taproot addresses **nobody holds a key to**.

Two things on staging are *not* simulated and are worth knowing: the **drand rounds** behind every draw are real, fetched from
`api.drand.sh` and verified on chain, and the **snapshots** are rebuilt byte for byte from the fork's own Transfer logs by the
same tool you can run yourself. `STAGING_SIM=0` turns the simulation off and leaves a quiet, empty chain.

---

## Tier 2: a small real test on Robinhood Chain (you run this)

> **Read this first.** This is real ETH on a real chain, in **unaudited** contracts. Whatever goes into a vault can leave only
> as a Bitcoin purchase: **there is no withdraw function**, by design. Fees a coin earns under 0.01 ETH wait in the vault for
> ever. Assume every wei you put in is spent. Do it with amounts you would not miss, tell nobody the addresses, and wind it down
> the same day. Nothing in this repository will run any of it for you: no script here holds a key, and no agent may do it.
> **Nobody has run this path.** It was prepared on the fork only.

### What cannot be made small

The limits are `constant` in the contracts (`MIN_ENTRY` 0.01 ETH, `MAX_OPEN` 3, the waits). **We did not change the contracts for
the test**, because a test of different bytecode is not a test of what would be deployed. So on the real chain the waits are the
real waits: 1 h before release, 15 min challenge, 1 h address correction.

**The Launcher is permissionless and cannot be restricted.** Anyone who finds the address can launch a coin against any
**unpaused** listing, and that coin's fees would flow into your test vault. The only lever is `setPaused(listing, true)`:
`Launcher.launch` reverts on a paused listing. So: keep every listing paused, unpause for the minute in which you launch, pause
again. **Residual risk:** during that minute a stranger could launch too, and a coin launched before the pause keeps paying fees
into the vault afterwards. The cost to you is nil (it is their money), but their fees would be stuck in an unaudited vault, which
is why the ribbon and the unlisted address matter. The default site never shows the test deployment.

### What it costs

Measured on the fork: the whole deployment (4 contracts, wiring, roles, two listings) is 8 transactions and 17.1 M gas. At the
gas price read from chain 4663 on 2026-09-21 (0.0556 gwei) that is **about 0.001 ETH**; budget **0.003 ETH** for the deployer.
Then: 0.0005 ETH Pons launch fee per coin; the trading money that generates fees (the vault gets 80% of the creator fee, and
needs 0.01 ETH of it before anything is spendable, which means a lot of volume: decide beforehand whether you want to go past
"launch, trade, harvest, split" at all); the purchaser's bond (the registry's `bondBps` share of what is released); a little gas
for each role. The keeper CLI refuses any declare above `KEEPER_TEST_MAX_WEI` (default 0.02 ETH).

### Steps

Use five fresh addresses you control (a hardware wallet for the owner and deployer). The owner must not be the deployer.

**1. Deploy, labelled `test`.** Writes `deployments/test-4663.json`, never `4663.json`, and refuses to overwrite.

```sh
ETCH_DEPLOY_CONFIRM=deploy-etch-test:4663:<deployer, lowercase> \
ETCH_OWNER=0x… ETCH_TREASURY=0x… ETCH_KEEPER=0x… ETCH_PURCHASER=0x… ETCH_ATTESTOR=0x… \
script/deploy.sh https://rpc.mainnet.chain.robinhood.com <deployer> --ledger --label test
```

Check: the file exists with `"label": "test"` and `"chainId": 4663`; `cast code <registry>` is not empty; `owner()` is your owner.

**2. Pause both listings at once** (owner): `cast send <registry> "setPaused(uint256,bool)" <id> true --ledger` for each id.
Check: `isPaused(id)` is true.

**3. Look at it.** Commit `deployments/test-4663.json`, build the site, open `https://etched.meme/?net=test`. A permanent ribbon
says *TEST DEPLOYMENT — real chain, tiny limits, unaudited*. Without `?net=test` the site still says no contracts are live.
The indexer for it: `DEPLOYMENT_LABEL=test`.

**4. Launch one coin.** Unpause one listing, launch from the site, pause again. Check `isPaused` is true again, and the coin
appears under `?net=test`.

**5. Trade it a little, then harvest.** Harvest and split are permissionless; the site or `cast` can call them. Check the vault
received 80% and the treasury 20% of what was claimed.

**6. (Only if the vault passed 0.01 ETH) the keeper and attestor calls, each by hand.** Every call needs all five facts, or it
refuses before signing: the mode, the latch, a confirm string naming chain and signer, the file labelled `test` naming that
signer in that role, and a node that says 4663.

```sh
cd keeper
npm run live -- --dry declare <listing> <maxWei> <qty> <capRef>        # prints the call, signs nothing
KEEPER_MODE=mainnet-test KEEPER_LIVE=1 KEEPER_CONFIRM=keeper-live:4663:<keeper, lowercase> \
  RPC_URL=https://rpc.mainnet.chain.robinhood.com  npm run live -- post-cap <listing> <capWei>      # with KEEPER_KEY in your shell
# attestor calls: ATTESTOR_MODE=mainnet-test ATTESTOR_LIVE=1 ATTESTOR_CONFIRM=attestor-live:4663:<attestor, lowercase>, ATTESTOR_KEY
```

The key (`KEEPER_KEY`, `ATTESTOR_KEY`) is set by you, in your own shell, for that one command. Nothing here stores, logs or makes one. Prefer `--dry`
and paste the call into your hardware wallet's tool. A real test of settle needs a real Bitcoin purchase by you. For a first
test, stopping after step 5 is a complete and honest result.

### Winding down

1. `setPaused(id, true)` on every listing (step 2). Nothing new can launch.
2. Any purchase declared and not released: owner `cancel(id)`. Any released and never settled: after its 3-day deadline the
   owner calls `writeOff(id, reason)` (the bond pays the coins; after 7 more days anyone may).
3. The purchaser calls `withdrawBond(amount)` once nothing of theirs is outstanding.
4. Delete `deployments/test-4663.json` from the repository and redeploy the site: `?net=test` then shows nothing.
5. What stays behind: the contracts (they cannot be destroyed), coins already launched, which go on paying fees into a vault
   nobody serves, and any ETH in a vault. That is the cost of the test, and the reason to keep it tiny.
