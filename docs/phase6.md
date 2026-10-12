# Phase 6: the legal pages, a staging chain anyone can click through, and the preparation for a small real test

Written 2026-09-21, before any of its code. Findings are appended at the end, each marked verified (how) or unverified.

Nothing in this phase sends a transaction to a public network, reads a real secret, or deploys to Robinhood Chain (4663). The
parent reviews and hosts what is prepared here; the user, and only the user, ever runs tier 2.

## Part A: the pages a product that touches money must have

Three pages in the site's own system, linked from the footer: **Terms of Use**, **Risk Disclosure**, **Privacy**. They describe
what this product does, not a generic one: coins that mostly go to zero, fees that buy an asset only when volume allows, a
purchase that happens off this chain by a bonded purchaser, every trusted party named as `docs/trust-model.md` names them,
deliveries that can be late, a venue that can cut access, unaudited code, no price claims, no promise of return, a creator who
receives nothing from fees, draws that may be restricted where the reader lives. **The operating entity and the governing law
are left as marked blanks.** Nothing is invented. Every page says it is not legal advice and that a lawyer must review it before
mainnet.

`docs/legal/prize-draw-memo.md` is for the user and their lawyer, not for the site: why a random draw of a valuable asset among
token holders can be read as a lottery or prize promotion, how prize, chance and consideration map onto this design, which modes
have no chance element, what levers reduce the risk and which do not, the securities-style and consumer questions a fee-funded
distribution raises, and the questions to put to counsel. It cites nothing it cannot verify, and says where it is unsure.

The X handle becomes a config value (`web/src/config.ts`), empty until the user creates the account.

## Part B: two tiers of testing, as CardPaired has

**Tier 1, staging: our copy of the chain.** A hosted, persistent anvil fork of Robinhood Chain with **chain id 9466301** (amended; see the end), so
that nothing signed for it can be replayed on 4663; the node refuses to start as 4663. Pons's real factory, curve and hook come
with the fork. Our contracts are deployed onto it by the same guarded wrapper (`deploy-etch:9466301:<deployer>`).

- `staging/chain/`: anvil behind a small proxy. The public gets JSON-RPC reads and `eth_sendRawTransaction`; **every admin
  namespace is blocked** (`anvil_`, `hardhat_`, `evm_`, `debug_`, `personal_`, `admin_`, `miner_`, `txpool_`, `engine_`), and so
  are `eth_sendTransaction`, `eth_sendUnsignedTransaction`, `eth_sign*` and `eth_accounts`, because the node behind it will send
  from any address it is asked to. `/faucet` (capped per address and per IP per day), `/healthz`, `/info` (chain id, fork block,
  the deployment file). The fork block is pinned at first boot and the state kept on a volume.
- **No key exists on staging at all.** The brief said to generate throwaway role keys in the container. A stricter thing is
  available: the role addresses are fresh random 20-byte addresses, which nobody holds a key to and which therefore cannot carry
  the sweeper delegations Phase 2 found on the well-known dev addresses, and the operator sends *as* them through the node's
  impersonation, reachable from inside the container only. There is nothing to leak. Decided differently from the brief; this is
  why.
- **One service for the chain and its operator, not two.** A separate `staging-keeper` service would need role credentials
  carried between services. The operator runs beside the node, where impersonation is local, so the services are
  `staging-chain` (node, proxy, bootstrap, operator) and `staging-api` (the indexer). Decided differently from the brief.
- **The public waits are hard-coded** (`DECLARE_DELAY` 1 h, `CHALLENGE` 15 min, `DESTINATION_CORRECTION` 1 h) and stay so: no
  mainnet constant is touched and no constructor grows a knob. On staging the operator shortens a wait *after it has been public
  for `STAGING_WAIT_S`* by rewriting the one stored deadline through the node's local admin method, exactly as the local
  end-to-end run does, because advancing the clock would pin drand rounds in the real future. The page shows the contract's own
  deadline, so a tester sees an hour's countdown cut short, and the ribbon says why.
- What is **real** on staging: Pons's contracts and their fees, every etched contract, the 80/20 split, the queue, the bond, the
  snapshot tool's root over the fork's real Transfer logs, drand rounds that did not exist when pinned and their on-chain
  verification, wallets signing real transactions for the staging chain. What is a **fixture**, labelled `staging-fixture` wherever it
  is recorded: prices, the ETH→BTC quote, the Bitcoin receipt, the delivery. `npm run staging:attest` lets the user record ONE
  real purchase they made themselves, by hand.
- The web app gains a staging mode (`?net=staging` or a wallet on the staging chain): a persistent ribbon, add-network, faucet, reads through
  the staging API, and a staging-only trade box on the coin page that buys and sells on the forked Pons curve so that fees
  exist to harvest. Every other network behaves exactly as today: no contracts are live.
- `npm run staging:smoke`: the whole cycle against a locally run copy of the stack, as a stranger would do it, through the
  public proxy with throwaway keys made in the test for the staging chain only; and the indexer's API is checked after each step.

**Tier 2, a small real test on 4663. Prepared, never run here.** The user deploys with their own hardware wallet and a little
ETH. The wrapper gains `--label test`: a different confirm string (`deploy-etch-test:4663:<deployer>`, so that neither string
can stand in for the other), a different file (`deployments/test-4663.json`, never `4663.json`), and the one relaxation a small
test needs: the owner may be a plain hardware-wallet address and need not be a multisig. **No contract changes.** The
contracts' limits are constants; what makes the test small is policy: tiny keeper-posted caps, a small bond, and listings the
owner keeps **paused** except while testing. The Launcher is permissionless but refuses a paused listing, so pausing is the
mitigation and its gap is stated in `docs/staging.md`: while a listing is unpaused for a test, a stranger can launch into it.
- Latches, as CardPaired and our own executor have them: a service that sends needs its mode, its latch, and a confirm string
  naming the chain and the signer: `KEEPER_MODE=mainnet-test`, `KEEPER_LIVE=1`, `KEEPER_CONFIRM=keeper-live:4663:<address>`,
  and a deployment file whose `label` is `test`. The same for the attestor. **There is no mode that sends to a 4663 deployment
  not labelled `test`.** The user sets the keys; nothing here generates or reads a real one. `keeper/src/` stays unable to send;
  the sending code is a separate directory with its own guard and tests.
- The web app reads `?net=test` behind a ribbon that says what it is. The default site still says nothing is live.

## How this could mislead

A fork is a copy at one block: prices, pools and Pons's operator's sweeping do not move on it, and a hosted fork depends on the
public RPC still serving that block's state, which is not verified for longer than this phase ran. Staging receipts are
fixtures: staging proves the contracts, the keeper's policy, the tools and the pages, and says nothing about whether a venue
fills an order, a bridge delivers, or an inscription arrives. Tier 2 is unaudited code holding real money, however little.

---

## Findings (appended 2026-09-21, after the work)

### Verified

- **A public Robinhood Chain testnet exists, and it is no use to us.** `https://rpc.testnet.chain.robinhood.com` answers
  `eth_chainId` with `0xb626` and had a head at block 122,461,242. The Pons factory and router addresses we use on 4663 have **no
  code** there (`cast code` empty on the testnet, 24 kB and 12 kB on mainnet). Whether Pons lives there at other addresses was
  not checked. So: a fork of mainnet is the only place the real Pons can be tried for free.
- **That testnet's chain id is 46630, the number the brief first gave our staging fork.** Found at the end of the phase and
  then fixed: see the amendment at the end (staging is 9466301, and every guard refuses 46630 by name).
- **The smoke test passed, 12 of 12, run for real** against a local copy of the stack through the public proxy, with the indexer
  beside it: admin and signing methods refused; faucet paid once and refused the second ask; a coin launched on the forked Pons
  factory by a throwaway wallet that signed for the staging chain; three curve buys; a harvest split 0.07992 / 0.01998 ETH (exactly 80/20);
  cap, declare, shortened wait, release, fixture settle charged to the coin that paid; the creator chose a weighted draw; the
  snapshot root rebuilt byte for byte from public logs (3 holders); **real drand round 20829126** pinned before it existed,
  resolved, verified on chain; the winner posted a destination; fixture delivery recorded (stage Delivered); the indexer's API
  showed the coin, the acquisition and the delivered draw.
- **Persistence:** the stack was stopped and started on the same data directory. No second bootstrap, same addresses, state kept.
- **The hard-coded waits** were handled with no contract change and no time travel: the operator rewrites one stored timestamp
  per wait on the fork, from localhost (`docs/staging.md` has the table). Real drand rounds stayed valid because the clock is real.
- **A bug the tests caught in our own proxy:** a refused faucet request (an address that had already been paid) still used up that
  IP's turn, so one refusal could lock a household out for a day. All three caps are now checked before any is taken.
- The site's CSP allowed only one loopback port, so a local staging stack was unreachable from the page; it now allows loopback.
  `sync-content.mjs` copied a local staging deployment into the bundle; it now bundles Robinhood Chain's files and nothing else.
- **Tier 2 cost, measured on the fork:** 8 transactions, 17,094,926 gas for contracts, wiring, roles and two listings. At the
  gas price chain 4663 reported (55,602,000 wei) that is 0.00095 ETH. Pons's `launchFee()` is 0.0005 ETH.
- The send guards: 4 tests (keeper), 11 (DeployChecks), 15 wrapper refusals, 5 (proxy and faucet), 9 (web, including that a file
  labelled `test` is never read as *the* deployment).

### Unverified

- **The Dockerfile was never built** (Docker was down). Everything it runs was run outside a container.
- Nothing was deployed to Railway; `staging/RAILWAY.md` is commands nobody has run.
- Whether the public RPC serves the pinned fork block's state for longer than a day.
- **Tier 2 was never run**, by design: not the `--label test` deploy on 4663, not one latched send. The guards are tested; the
  path is not.
- The operator's rune / pro-rata path (`settleRune`, `openDistribution`, `recordPayout`) typechecks and was never exercised.
  Named, Hold and Never were not exercised on staging either; the smoke test covers the weighted draw.
- The web staging mode was checked with headless screenshots against the local stack (figures, ribbon, trade box rendered). No
  real wallet clicked anything: add-network, the trade box's sends and the choose control are unverified in a browser wallet.
- The legal pages and the memo are our own plain-language drafts. No lawyer has seen them, and the memo cites no law because
  none could be checked.
- **CI caught one of ours after the first push:** the indexer's own guard ("it contacts the chain RPC and nothing else") failed,
  because the staging indexer fetched the deployment from a second configured URL, and the indexer's tests had not been rerun
  after that change. Fixed by keeping the rule: `DEPLOYMENT_FROM_RPC=1` asks the RPC's **own host** for `/info`, and the guard
  test now allows exactly that one line in `main.ts`. Checked against a stub node.

### Amendment, 2026-09-21: the staging chain id is 9466301

The plan above first said 46630 (the brief's number) and has been corrected in place. The coordinator's ruling on the clash found above: staging must share an id
with no real network. It is now **9466301**, named *etched staging*, currency **sETH**. One source, `staging/chain-id.json`:
read directly by the staging package, the keeper's staging guard and `script/deploy.sh`; mirrored by exactly one constant in
`script/DeployChecks.sol`, `indexer/src/main.ts` and `web/src/lib/chains.ts`, each held to the file by a test (their images are
built without `staging/`). The boot assertion, the bootstrap, the keeper guard and the wrapper refuse 4663 and 46630 by name and
anything that is not the constant. The smoke test was run again in full on the new id: result below.

### Two defects the second smoke run found, both of which would have broken a hosted fork within the hour

Neither has to do with the chain id. The first run passed because it began seconds after a first boot; the second began ten
minutes after, and failed at the faucet.

1. **The public RPC keeps about a quarter of an hour of historical state** (measured 2026-09-21: a balance read 6,000 blocks back
   answered; 9,000 back, and everything older, was refused with "historical state … is not available"). A fork pinned at an older block
   cannot read any account or slot it has not already cached, and cannot even `anvil_setBalance` a fresh address. The plan listed
   this as unverified; it is now verified, and it is worse than feared. **Fix:** anvil now reads the real chain through
   `staging/chain/upstream.ts`, a shim on 127.0.0.1 that forwards reads only (an allowlist; tested) and, when a state read is refused as too
   old, asks again at `latest`. Cost, stated in `docs/staging.md`: state the fork never touched is read as it is on Robinhood Chain
   *now*, so a contract could in principle be seen at two moments. `/healthz` counts these (`upstreamFallbacks`). An archive node
   in `STAGING_UPSTREAM` avoids it; we know of none that needs no key.
2. **A restarted fork lives in the past.** Anvil's clock starts at the fork block's time, so after a restart the chain was 20
   minutes behind the wall clock: the operator's waits never met the contracts', and, worse, a draw would have pinned a drand round
   that **already existed**. **Fix:** at every boot the supervisor brings the chain to the present (`evm_setNextBlockTimestamp`
   to now, never backwards; it holds afterwards, checked).

Also: the faucet's default of 3 addresses a day from one place could not fund the smoke test's four wallets; it is 5. And
`staging/RAILWAY.md` told the parent to look for a `/healthz` field that does not exist (`bootstrapped`); it is `deployed`.
3. **A one-member collection.** The demo listing's membership root was one inscription id, and an inscription can be held once,
   so a long-lived staging chain could complete exactly one inscription purchase, ever: the second failed with `AlreadyHeld`
   (reported by anvil, misleadingly, as "insufficient funds"). On the staging chain only, the collection now has 1,024 fixture
   members; the operator settles with the next one nobody holds and its real Merkle proof. The root is built twice
   (`script/Deploy.s.sol`, `keeper/staging/members.ts`) and pinned to one value in both test suites.
4. **The bootstrap starved its own node.** With the shim in the supervisor's process, the synchronous deploy blocked the event
   loop the node's reads were waiting on, and a first boot timed out. The deploy is now asynchronous, and a failed bootstrap
   kills the node instead of leaving it behind.
