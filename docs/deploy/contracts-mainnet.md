# Contracts on Robinhood Chain (4663): the go-live runbook

Written 2026-09-23 at commit `golive-prep` (merged with `main` at `01a1e5d`). Nothing here has been run against 4663. Every
command was rehearsed on local nodes by `scripts/golive-dryrun.sh`, including a local fork of 4663 itself (results at the end).

The operator's part is to run reviewed commands with a hardware wallet — or, for a solo operator who has none, a password-encrypted
Foundry keystore (added 2026-09-25, below) — and a multisig. No command here reads a private key or a password, and
every script refuses to act on 4663 unless the chain, the sender and — where it matters — the address that can never be
changed afterwards are typed out by hand.

## Checked first: `ListingRegistry` is over Ethereum's contract-size limit — not a blocker on 4663

`ListingRegistry` is **41,353 bytes** of runtime code (40,845 before the NFT buyer-reservation hook, docs/v4/nft.md §2.1; 39,540 before the 2026-09-23 single-clock fix) (it embeds both vaults' creation code), over EIP-170's 24,576. The buyer-loop
work raised it as a deploy blocker; it was measured on 2026-09-23 and **Robinhood Chain accepts it**:

- **Published:** Robinhood's own docs — *"a maximum code size of 96 KB (vs. Ethereum's 24 KB) and a max init code size of 192 KB"*
  ([Differences from Ethereum](https://docs.robinhood.com/chain/differences-from-ethereum/)). 96 KB is the ceiling an Arbitrum chain
  may configure (`MaxCodeSize` up to 98,304, `MaxInitCodeSize` up to 196,608), and it *"cannot be modified after deployment"*
  ([Arbitrum: configure smart contract size limit](https://docs.arbitrum.io/launch-arbitrum-chain/chain-config/execution/smart-contract-size-limit)).
- **Measured on the real chain, read-only:** `eth_estimateGas` on `https://rpc.mainnet.chain.robinhood.com` for the real
  `ListingRegistry` creation code answers **8,729,479 gas** (no error). A CREATE returning 98,304 bytes is accepted; 98,305 is refused
  with *"max code size exceeded: code size 98305 limit 98304"*.
- **On an anvil fork of 4663 without any size flag** it is refused: `EVM error CreateContractSizeLimit`. anvil applies Ethereum's
  default, not the forked chain's, so a local rehearsal must be told the chain's limit — which is all `--code-size-limit 98304` does
  in `scripts/golive-dryrun.sh` and the deploy scripts. The dry runs never used `--disable-code-size-limit`: they enforce 98,304, the
  real limit, and would still refuse a contract over it. A local test started with `--disable-code-size-limit` proves less.

`forge build --sizes` after the single-clock fix (docs/audit/README.md, 2026-09-23; the figures at `1e8dfe8` were ListingRegistry 39,540, AssetVault 15,037, Distributor 13,975, AutoBuyerVault 13,744):

| Contract | Runtime (B) | Initcode (B) | Runtime margin vs Ethereum's 24,576 | Runtime margin vs 4663's 98,304 |
| --- | --- | --- | --- | --- |
| ListingRegistry | 41,353 | 41,715 | **−16,777** | 56,951 |
| AssetVault | 15,312 | 15,675 | 9,264 | 82,992 |
| Distributor | 14,687 | 14,943 | 9,889 | 83,617 |
| AutoBuyerVault | 14,089 | 14,587 | 10,487 | 84,215 |
| Wrapper | 9,469 | 10,918 | 15,107 | 88,835 |
| WrapClaim | 5,074 | 16,559 | 19,502 | 93,230 |
| Launcher | 3,835 | 7,019 | 20,741 | 94,469 |
| DrandEvmnet | 3,578 | 3,604 | 20,998 | 94,726 |
| FeeRouter | 2,680 | 2,744 | 21,896 | 95,624 |
| Treasury | 2,015 | 2,171 | 22,561 | 96,289 |

**Where it would become a blocker, and the fixes if it does:** on any chain with Ethereum's limit (Ethereum, Arbitrum One, Base), or
on 4663 if the registry grows past 98,304. Options, cheapest first — none is taken here; the decision is the coordinator's:

1. **Vault factories.** Move `new AssetVault(...)` / `new AutoBuyerVault(...)` into two small factories the registry calls (each
   carries one vault's creation code, ~15 KB); the registry drops to roughly 10 KB. Each factory must be callable only by the
   registry, or anyone can mint look-alike vaults `vaultOf` does not know. Deployment grows by two CREATEs and a wiring step, and
   `golive-check.sh` must read the factories back.
2. **CREATE2 from a factory**, salted by listing id: as 1, plus vault addresses known before listing — which would also remove the
   canister runbook's placeholder-then-reinstall step ("The vault argument").
3. **Clones of implementation addresses** (EIP-1167, as `FeeRouter` already is): the smallest registry and cheapest listings, but the
   vaults' `immutable`s (`registry`, `listingId`, `isRune`, `buyer`, `maxInFlight`) become initialised storage — reopening
   initialiser-front-running and "is this the real implementation" questions. The largest change to the audit surface.

## What goes on chain, in order

| Step | What | Who sends | How | Changeable afterwards? |
| --- | --- | --- | --- | --- |
| 1 | `ListingRegistry(owner, treasury)` | deployer (hardware wallet or keystore) | `script/deploy.sh` | owner: `Ownable2Step` transfer, **no delay**; treasury: `proposeTreasury` + 7 days |
| 1 | `DrandEvmnet()` — the drand beacon | deployer | `script/deploy.sh` | no (a new one means a new Distributor) |
| 1 | `Launcher(PONS_FACTORY, registry)`, which deploys the `FeeRouter` implementation | deployer | `script/deploy.sh` | no |
| 1 | `Distributor(registry, beacon)` | deployer | `script/deploy.sh` | no |
| 2 | `registry.wire(launcher, distributor)` | **multisig** | calldata printed by step 1 | **no — once, forever** |
| 2 | `registry.proposeRoles(keeper, purchaser, attestor)` | multisig | calldata printed by step 1 | yes, `proposeRoles` + 7 days (the first set applies at once) |
| 3 | the buyer canister goes live | operator | `docs/deploy/canister-mainnet.md` | — |
| 4 | `registry.listInscriptionsAuto(…, buyer, maxInFlight)` → the registry creates an `AutoBuyerVault` | multisig | `script/list-auto.sh` prints the calldata | **buyer: never.** root, BTC vault: `propose…` + 7 days |
| 5 | read everything back | anyone | `script/golive-check.sh` | — |
| 6 | `WrapClaim(registry, distributor, beacon, listingId, custodian, custodyAddress, name, symbol)`, which deploys its own `Wrapper` | deployer | `script/deploy-wrapclaim.sh` | no |
| 7 | read back again, with the wrapper pair | anyone | `script/golive-check.sh … deployments/wrapper-4663-<id>.json` | — |
| 8 | explorer verification of every contract | anyone | below | — |
| (9) | `Treasury(registry)`, then `proposeTreasury` → 7 days → `acceptTreasury` → `setToken` | deployer, then multisig | `script/DeployToken.s.sol` | — **legally gated**: its header says counsel must answer `docs/token/protocol-token-v1.md` §5 first |

Two orderings are forced, not chosen:

- **The canister before the listing (3 before 4).** An `AutoBuyerVault`'s `buyer` is immutable: there is no key anybody can
  rotate to point the money elsewhere (`src/AutoBuyerVault.sol`). The buyer is the canister's threshold-key EVM address, which
  exists only once the canister is installed on the IC. A listing named with anything else is a vault that pays the wrong
  account forever.
- **The listing before the wrapper (4 before 6).** `Wrapper`'s constructor reads `registry.vaultOf(listingId)` and refuses a rune
  vault; `GoLiveChecks` additionally refuses, on 4663, a wrapper over a bonded (v1) vault — the custodian is the v2 buyer.

## Before anything

1. **The audit is done** (`docs/audit/README.md`), and its findings are fixed or accepted in writing. *For Phase 1 the operator
   waived this (G1) for stage 0 only, in writing, on 2026-09-24 (docs/v6/phase1-scope.md §2); the site's Trust page says so.*
2. **The owner is a Safe on 4663.** Safe's v1.4.1 contracts are deployed there (checked 2026-09-23: `SafeL2`
   `0x29fcB43b…0C762` and the proxy factory `0x4e1DCf7A…0ec67` have code). Create the Safe with signers on separate hardware
   wallets held by separate people (`docs/deploy/key-ceremony.md`). **Not a single-key account**: `script/Deploy.s.sol` refuses an
   owner with no code, and now also an account whose only code is an EIP-7702 delegation (`0xef0100 ‖ delegate`, 23 bytes).
   EIP-7702 is live on 4663 — the well-known dev accounts carry sweeper delegations there (`docs/phase2.md` F2) — and the owner
   check had not accounted for it: a single-key account with a delegation would have passed as a multisig.
3. **Five distinct addresses, none of them the deployer or the owner:** treasury, keeper, purchaser, attestor, and later the
   buyer. The registry refuses role overlap on chain (`RolesNotDistinct`, `SameKey`), and `DeployChecks` refuses it before
   anything is sent.
4. **The deployer** is a hardware wallet — or a Foundry keystore ("The deployer's signer", below) — holding a little ETH and
   nothing else. It owns nothing after step 1.
   Budget: the step-1 CREATEs measured **28.1M gas** on the fork; forge's estimate at the 2026-09-23 base fee (0.056 gwei) is
   **0.0031 ETH**. A local fork does not charge Arbitrum's L1 data fee, so hold **0.02–0.03 ETH** and read the real figure from
   the node before sending (`cast estimate` / the hardware-wallet screen).
5. **Rehearse on this machine, on the day:**
   ```
   forge test --no-match-path "test/fork/*"          # 278 tests at 9333e29
   script/deploy.test.sh                             # the deploy script's refusals
   scripts/golive-dryrun.sh all                      # bare node, fork of 4663, refusals on 4663/46630: must end "DRY RUN: ALL PASS"
   ```

## Why every script passes `--code-size-limit 98304`

See "Checked first" above: it tells forge and anvil 4663's real limit, which they do not read from the chain.

## Step 1: the system

```
ETCH_DEPLOY_CONFIRM=deploy-etch:4663:<deployer, lowercase> \
ETCH_OWNER=<safe> ETCH_TREASURY=<treasury> ETCH_KEEPER=<keeper> ETCH_PURCHASER=<purchaser> ETCH_ATTESTOR=<attestor> \
script/deploy.sh https://rpc.mainnet.chain.robinhood.com <deployer> --ledger      # or --account <keystore>: below
```

The script asks the RPC which chain it is before forge starts, refuses a key in the environment, requires `--ledger`,
`--trezor` or `--account <keystore>` on 4663, and asks for the chain id to be typed. It writes `deployments/4663.json` (never overwriting one) with the
chain's own `fromBlock`, and prints the two calls for the Safe. Commit the file.

### The deployer's signer: a hardware wallet, or a keystore (2026-09-25)

The operator has no hardware wallet (docs/v5/launch-gates.md G2 rule 5; docs/v6/phase1-scope.md K8, O5), so `script/deploy.sh`,
`script/deploy-stack.sh` and `script/deploy-wrapclaim.sh` also take a Foundry keystore on 4663:

```
cast wallet new ~/.foundry/keystores rook-deployer        # once, on device A: a hidden password prompt; prints the address only
ETCH_DEPLOY_CONFIRM=deploy-etch:4663:<that address, lowercase> … script/deploy.sh https://rpc.mainnet.chain.robinhood.com <that address> --account rook-deployer
```

Before forge starts the script refuses, on top of every refusal above:

- a keystore name that is not letters, digits, dot, dash or underscore (starting with a letter or digit);
- a keystore that is not at `~/.foundry/keystores/<name>`: that is the only place forge's `--account` reads, whatever
  `FOUNDRY_KEYSTORES_DIR` says (measured 2026-09-25), so a `FOUNDRY_KEYSTORES_DIR` naming another directory is refused too — the
  file checked must be the file that signs;
- a keystore whose address is not the deployer's. The address is the one the file records; if it records none, cast opens the
  keystore once (the operator types the password) and names it;
- `ETH_KEYSTORE`, `ETH_KEYSTORE_ACCOUNT` or `ETH_PASSWORD` in the environment (forge would read a signer or a password nobody typed);
- two signer flags; and, on our own chains, any signer flag at all.

The chain id is then typed, the prompt names the keystore, and forge asks for the password when it signs. The key is never on a
command line, in the environment or in a file the script reads. `script/deploy.test.sh` and `scripts/golive-dryrun.sh refusals`
hold every refusal above, and show the deployer's keystore accepted up to the typed chain id with nothing sent.

## Step 2: the Safe wires the registry, once

Propose, in the Safe, the two calls step 1 printed, to the registry: `wire(launcher, distributor)` and
`proposeRoles(keeper, purchaser, attestor)`. Check the decoded calldata in the Safe's own view against `deployments/4663.json`
before signing. **`wire` can be made once**: a wrong address here makes the registry useless and the only remedy is redeploying
step 1. Then:

```
script/golive-check.sh https://rpc.mainnet.chain.robinhood.com deployments/4663.json     # must end ALL CHECKS PASS
```

## Step 3: the buyer canister

`docs/deploy/canister-mainnet.md`, which ends with the buyer's EVM address read from the IC and written to
`deployments/canister-ic.json`.

## Step 4: a listing that names the buyer

```
ETCH_REGISTRY=<registry> ETCH_LIST_KIND=inscriptions ETCH_LIST_NAME="Bitcoin Frogs" ETCH_LIST_REF=bitcoin-frogs \
ETCH_LIST_ROOT=<membership root> ETCH_LIST_BTC_VAULT=<the canister's taproot address> \
ETCH_BUYER=<evm_address() from the IC> ETCH_BUYER_CONFIRM=buyer-is:4663:<that address, lowercase> \
ETCH_MAX_IN_FLIGHT=<wei> script/list-auto.sh https://rpc.mainnet.chain.robinhood.com
```

On 4663 **this sends nothing**: it simulates the call and prints `to`, `owner`, the listing id it will get, and the calldata.
The Safe proposes it. `GoLiveChecks` refuses a buyer that has code (a contract, or a delegated account), a buyer confirm typed for
another address or chain, a missing root or BTC vault, and `maxInFlight = 0`; the registry itself refuses a buyer that is the
owner or holds a role. `maxInFlight` is the most the vault may have announced-or-pulled and unsettled at once: start small.

## Step 6: the wrapper pair

```
ETCH_WRAPPER_CONFIRM=deploy-etch-wrapper:4663:<deployer, lowercase> ETCH_REGISTRY=<registry> ETCH_WRAPPER_LISTING=<id> \
ETCH_WRAPPER_NAME="Wrapped Bitcoin Frog" ETCH_WRAPPER_SYMBOL=wFROG \
ETCH_WRAPPER_CUSTODIAN=<the canister's evm_address()> ETCH_WRAPPER_CUSTODY_ADDRESS=<the first word of its custody_address()> \
script/deploy-wrapclaim.sh https://rpc.mainnet.chain.robinhood.com <deployer> --ledger      # or --account <keystore>
```

**Custody first** (docs/v3/wrapper.md, "Custody on its own key"). The canister holds frogs at a key of its own, derivation path
`custody`, never at the buyer's address: read it with `custody_address()` (and compare with `buyer_address()` — they must
differ). Before step 6 the listing's `btcVault` must already **be** that custody address, because purchased inscriptions are
delivered to `btcVault` and `wrap` refuses while the two differ; if the listing was created with another address, move it with
`proposeBtcVault` / `acceptBtcVault` (7 days) first. `ETCH_WRAPPER_CUSTODIAN` is the canister's `evm_address()`: the Wrapper
lets it record its own deliveries (`recordDelivery`), and nothing else. Both are fixed in the constructor: a wrong one means a
new WrapClaim, not an edit. Fund the custody address with a small float of **plain** sats (no inscriptions, no runes; ~20,000
sats covers dozens of deliveries): each delivery's fee comes only from a plain custody UTXO.

One CREATE: `WrapClaim` builds its `Wrapper` in its constructor and names itself the Wrapper's `distribution`
(`src/WrapClaim.sol`). The script reads the Wrapper back from `wrapClaim.wrapper()`, checks the two point at each other and at
the registry's own Distributor and beacon, and writes `deployments/wrapper-4663-<id>.json`. It refuses a Distributor or beacon
other than the registry's, a rune listing, and on 4663 a bonded (v1) listing — and a zero custodian, a custody address that is
not taproot, or one that differs from the listing's `btcVault` (`GoLiveChecks.checkWrapper`). It reads back `custodian()` and
`custodyAddress()`; `golive-check.sh` reads both again, and that the listing's `btcVault` equals the custody address.

## Steps 5 and 7: read everything back

```
script/golive-check.sh https://rpc.mainnet.chain.robinhood.com deployments/4663.json deployments/wrapper-4663-<id>.json
```

Read-only (`cast call`). It checks every immutable and role against the file: the registry's owner, treasury, launcher,
distributor, keeper, purchaser, attestor; the Launcher's factory, registry, escrow (equal to Pons' own `feeEscrow()`) and router
implementation; the implementation's `launcher` and its burned `coin` sentinel; the Distributor's registry and beacon; for every
listing its vault's registry, id and kind, and for a v2 vault that the registry knows its buyer and that the buyer has no code;
and for the wrapper pair, both directions of the pairing. Exit 1 on any mismatch.

With a stack file (docs/v6/deploy-stack.md) it also reads every stack wiring back, compares each listing's `btcVault` with its
wrapper's `custodyAddress()` (as it does for a wrapper pair), and with `--canister <buyer id>` prints the buyer's `custody_caps()`
beside the stack's stage and fails if a collection's cap is not the stage's (2026-09-25, K9).

## Step 8: explorer verification

Robinhood Chain's explorer is Blockscout at `https://robinhoodchain.blockscout.com` (where
`explorer.mainnet.chain.robinhood.com` redirects). Its API sits behind a Cloudflare browser challenge (seen 2026-09-23), so
`forge verify-contract` may be refused from a terminal; the fallback is the explorer's own "Verify & Publish" page with the
standard JSON input forge prints with `--show-standard-json-input`. Compiler: solc 0.8.28, evm `cancun`, optimizer 200 runs,
**via-IR** (`foundry.toml`) — a verification without `--via-ir` will not match.

| Contract | Deployed by | Constructor arguments |
| --- | --- | --- |
| `src/ListingRegistry.sol:ListingRegistry` | deployer | `(owner, treasury)` |
| `src/rand/DrandEvmnet.sol:DrandEvmnet` | deployer | none |
| `src/Launcher.sol:Launcher` | deployer | `(0x7eD598BcEf8bd9Edd8C97A195C6d13f40801EC7e, registry)` |
| `src/FeeRouter.sol:FeeRouter` (implementation) | the Launcher | none |
| `src/Distributor.sol:Distributor` | deployer | `(registry, beacon)` |
| `src/AutoBuyerVault.sol:AutoBuyerVault` (one per v2 listing) | the registry | `(registry, id, isRune, divisibility, buyer, maxInFlight)` |
| `src/WrapClaim.sol:WrapClaim` | deployer | `(registry, distributor, beacon, listingId, custodian, custodyAddress, name, symbol)` |
| `src/Wrapper.sol:Wrapper` | the WrapClaim | `(registry, distributor, beacon, listingId, wrapClaim, custodian, custodyAddress, name, symbol)` |

```
forge verify-contract <address> <path>:<Name> --chain 4663 --via-ir --verifier blockscout \
  --verifier-url https://robinhoodchain.blockscout.com/api/ \
  --constructor-args $(cast abi-encode "constructor(address,address)" <owner> <treasury>)
```

Router clones are EIP-1167 proxies of the implementation; Blockscout links them once the implementation is verified.

## If something goes wrong

Nothing here can be undone by the deployer, who owns nothing. What is available, in the order it would be needed:

| Found | Before money moves | After |
| --- | --- | --- |
| a wrong address in `deployments/4663.json` or a failing `golive-check` after step 1 | abandon: do not wire. Deploy step 1 again; the old contracts own nothing and hold nothing | — |
| a wrong `wire` | redeploy step 1 (`wire` is once) | — |
| a wrong role | `proposeRoles` again; applies after 7 days (the first set applied at once) | the same, and `setPaused` the listings meanwhile |
| a wrong buyer in a listing | `setPaused(id)` and never fund it; list again with the right buyer. The vault can never be fixed | `setPaused(id)`; the vault's own deadline and return paths release what it holds |
| a wrong root or BTC vault | `proposeRoot` / `proposeBtcVault`, 7 days | the same, paused meanwhile |
| a wrong WrapClaim | deploy another; nothing wraps into the first unless someone calls it | `setPaused(listing)` stops new wraps; redeem, resolve, deliver and refund cannot be paused (`docs/v3/wrapper.md`, decision 6) |
| the buyer canister misbehaving | — | the guardian pauses it through the timelock (`docs/deploy/canister-mainnet.md`) |

## Rehearsal results (2026-09-23)

`scripts/golive-dryrun.sh all`, port 8604, every node local, every sender an anvil dev account: **ALL PASS**.

- **Bare node (31337)**, with `script/local/LocalPons.sol` planted at Pons' address: steps 1–7, every read-back `ok`, and seven
  refusals: a buyer confirm for another address, a buyer that is a contract, a buyer that is the owner (the registry's
  `SameKey`), `maxInFlight = 0`, a wrapper with no confirm, a wrapper over a rune listing, a key in the environment.
- **Fork of 4663** (block ~70.14M, real Pons factory, under our staging chain id 9466301 so nothing signed there could be replayed
  on 4663): the same, all `ok`. Two things only the fork showed: the code-size limit above, and the dev accounts' EIP-7702
  delegations (`docs/phase2.md` F2) tripping the buyer check (so the rehearsal's buyer is `0x…b0b0e1`, which has no code on 4663).
- **A node reporting 4663**: `deploy.sh` and `deploy-wrapclaim.sh` refuse without a hardware wallet (since 2026-09-25: without a
  hardware wallet or the deployer's own keystore, see "The deployer's signer"); `list-auto.sh` prints
  "NOTHING WAS SENT" and the owner's nonce and the registry's listing count are unchanged. **A node reporting 46630** (Robinhood's
  public testnet): both scripts refuse.

## When the other branches land

- **The buyer loop** (agent 1): if the loop reads the listing's vault from the install argument, step 3 must happen with the
  vault known — see `docs/deploy/canister-mainnet.md`, "The vault argument". If the loop serves several vaults, the single `vault`
  field in `Init` is not enough and this runbook's step 4 repeats per listing.
- **Anything that changes a constructor** changes step 8's table and `script/golive-check.sh`.
