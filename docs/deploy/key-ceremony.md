# The keys: who holds what, what each can and cannot do, and what happens when one is lost or stolen

Written 2026-09-23. No key was generated or handled in writing this. The canister's own signing keys are threshold keys held by an
IC subnet; **nobody holds them**, and nothing here changes that. Every key below is a person's, and the point of the ceremony is
that no one person, machine or key can move money or change code alone.

## The keys

| Key | Holds | Where it lives | Used |
| --- | --- | --- | --- |
| **Safe signers** (e.g. 3 of 5) | the contracts' owner: `wire`, roles, listings, pause, `cancel` in WrapClaim's 15 minutes | one hardware wallet each, held by **different people** in different places | for every owner action, through the Safe |
| **Deployer (EVM)** | nothing after its deployments (contracts runbook steps 1 and 6, and the stack) | a hardware wallet with a little ETH, **or** (the solo operator, who has none: 2026-09-25) a password-encrypted Foundry keystore on device A, `~/.foundry/keystores/<name>` — see "The deployer as a keystore" below | once per deployment |
| **Keeper, purchaser, attestor** | the v1 roles (`docs/trust-model.md`) | each its own key, on the machine that runs that role | continuously, by services |
| **Proposers (IC)**, 2 or more | any one may propose an upgrade, a controller change or a roles change (one pending each; may withdraw only its own) | each an HSM-backed `dfx` identity (`dfx identity new <name> --hsm-pkcs11-lib-path <lib> --hsm-key-id <id>`: a YubiHSM or Nitrokey through PKCS#11) on its own otherwise-offline machine, **stored apart** from the others | rarely: each upgrade |
| **Guardians (IC)**, 3 or more, veto k = 2 | k of them veto a proposal; any one pauses the buyer; k resume it | each an HSM-backed identity held by **a different person** (none of them a proposer), reachable within hours | on an alert |
| **Operator (IC)** | run the buyer: `tick`, `open_purchase`, `record_quote`, `sign_purchase` — every order and quote checked against the chain (`docs/v2/trust-points.md` §1); pause (not resume) | a hot identity on the server that runs the loop | continuously |
| **Deployer (IC)** | the buyer and timelock until the handover and the blackhole | a throwaway identity | one sitting; destroyed after the blackhole |
| **Watcher's Telegram bot token** | posting to the alert chat | the watcher host's environment | continuously; low value |

Rules that the code enforces: the production timelock refuses fewer than 2 proposers or 3 guardians, a veto threshold outside
`2 ≤ k ≤ guardians − 1`, and any principal in both roles (`docs/v2/trust-points.md` §3); the registry refuses a buyer or role that
equals the owner or another role (`RolesNotDistinct`, `SameKey`); `script/Deploy.s.sol` refuses an owner with no code **and an
owner whose only code is an EIP-7702 delegation** (a single key, not a multisig — added 2026-09-23). Rules it cannot enforce, from
`docs/trust-model.md`: **every EVM role address must be fresh and empty of code** — on Robinhood Chain the well-known development
addresses carry EIP-7702 delegations to a sweeper that takes any ETH sent to them (`docs/phase2.md` F2) — and the guardian must be a
different person from the proposer, which the code can check only as a different principal.

## What each can and cannot do

From `docs/v2/timelock-controller.md` and the contracts:

- **A proposer** can propose an upgrade, a controller change or a roles change; nothing takes effect for 72 hours, the module hash
  and the argument are public during the delay, and k guardians can veto. It can withdraw only its own proposal. It cannot run
  anything sooner, cannot pause, cannot operate.
- **A guardian** can vote to veto a pending proposal (k votes veto it), pause the buyer alone, and vote to resume it (k votes
  resume it). It cannot propose, cannot move funds, and cannot make the buyer do anything but stop or start.
- **The operator** can do what the buyer's code allows an operator to do, immediately: open purchases, run the loop, record
  quotes, have verified PSBTs signed, pause. Every order and quote must match the chain: the listing's rune and Bitcoin address,
  the vault's units, and a spend within 3% of the released ETH's reference value. It cannot upgrade, cannot resume after a pause,
  and cannot change the timelock.
- **The Safe** can do everything owner-only on the contracts. The registry's loosenings wait 7 days (roles, roots, BTC vaults,
  treasury); `wire` once; pause listings at once. **Not delayed** (corrected 2026-09-25, docs/v6/phase1-scope.md §7 item 4):
  `ProtocolTreasury.setShares`, `setOps`, `setBootstrapLoan` and `addCollection` take effect at once, and ownership moves by
  `Ownable2Step` with no waiting period. It cannot change a vault's buyer, ever, and cannot touch the canister.
- **Anyone** can execute a proposal once its delay has passed, read `status()` and `history()`, and verify every module hash.

## Lost or stolen: what happens, and what to do

| Key | Stolen | Lost |
| --- | --- | --- |
| **Operator** | **No longer custody** (`docs/v2/trust-points.md` §1). The attacker can open purchases (within the keeper's cap, `maxInFlight`, `MAX_OPEN`), choose among honest quotes (paying the listing's own address, at least the announced units, within 3% of the released ETH's reference value), refuse to act, abandon a purchase before any BTC is spent, and pause. The worst it can take, colluding with a venue, is that 3% plus overpaying within the cap. **Do:** a guardian pauses the buyer (`pause_target`; the operator cannot resume), a proposer proposes an upgrade whose argument names a new operator; it lands after 72 hours; k guardians resume. | The buyer stops acting. Purchases in flight run into the vault's deadlines, whose return and write-off paths release the ETH. Rotate as for theft, without the pause. |
| **One proposer** | It can propose; each proposal needs k guardians to veto (the watcher posts it). It cannot withdraw anyone else's proposal, and its one pending slot does not block the others'. An honest proposer proposes `propose_roles` without it; the stolen key cannot withdraw it and one guardian cannot veto it; it lands after 72 hours. **Bounded by one delay.** | The other proposers carry on; one of them proposes `propose_roles` replacing it. |
| **One guardian** | It cannot veto alone (k = 2). It can pause the buyer, repeatedly: a denial of service, not a theft, until a `propose_roles` removes it, which it cannot veto alone. **Bounded by one delay.** | The veto still needs only k of the remaining guardians (k ≤ guardians − 1). A `propose_roles` replaces it. |
| **k guardians together** | They can veto every proposal, including the roles change that would remove them, and keep the buyer paused: its code is frozen and it may be stopped. They still cannot move funds or change code. | — |
| **Every proposer** | — | No upgrade and no roles change, ever; the buyer runs as it is. Hence 2 or more, stored apart. There is deliberately no recovery through the guardians: a veto set that can install a proposer is no longer only a veto. |
| **A proposer and k guardians together** (collusion) | The delay still makes any change public for 72 hours; nothing stops it. This is the design's stated limit. | — |
| **A Safe signer** | Below threshold: remove it from the Safe with the others. At or above threshold: the owner is theirs — roles, listings, pause, `cancel` — but no vault's buyer and no canister. | Replace it with the others, as Safe owners. |
| **Deployer (EVM)** | Nothing it owns: it owns nothing after its deployments. Before or during a deployment, a thief with the key (a keystore needs its file **and** its password) could deploy look-alike contracts from it; `golive-check.sh` reads every address back, and the Safe wires only what the log names. | Nothing. |
| **Deployer (IC)** | Before the handover: **everything** — it can install any code on the buyer. So the handover and the blackhole happen in the same sitting as the install, and the identity is destroyed after. After the blackhole: nothing. | Before the handover: create new canisters (new ids mean new threshold keys and new addresses, so nothing can have been funded yet). After: nothing. |
| **Keeper** | Bounded by `fixes-B-C-keeper-draws.md` §3 (a walked floor within ±20 % a day, sweeps up to the ceiling, forged draw roots caught by the monitor's recomputation). **Do:** the guardian or the Safe `pauseKeeper()` (instant, everywhere) → `cancelDraw(id)` for every unpinned draw → `proposeRoles(new keeper, …)` → 7 days → `acceptRoles()` → `unpauseKeeper()` (`docs/v5/launch-gates.md` §4). Burns continue throughout. | The module goes stale after an hour (mint, fills, sweeps refused); after 7 days burns pay the backing with no floor. Rotate as for theft, without the pause. |
| **Guardian** (optional, the monitor's key) | It can only pause the keeper and cancel draws before their pin: denial of service, never a theft. **Do:** the Safe `setGuardian(<new key>)` (or `address(0)`) and `unpauseKeeper()`. | Nothing stops; only the Safe can pause. Replace it with `setGuardian`. |
| **Purchaser / attestor** | `docs/trust-model.md` bounds each; rotate with `proposeRoles` (7 days) and pause the listings meanwhile. | Rotate the same way. |
| **Telegram bot token** | Someone can post to the alert chat, or read it. Revoke it in BotFather and issue a new one. | The same. |

No single key, lost or stolen, can freeze or redirect the buyer for longer than one delay (`docs/v2/trust-points.md` §3). What
remains is collusion — k guardians together, or a proposer with k guardians — and losing every proposer. The mitigation for those
is custody: HSMs, different people, keys stored apart, and the watcher.

## The deployer as a keystore (2026-09-25, docs/v6/phase1-scope.md §8.1)

The solo operator has no hardware wallet (docs/v5/launch-gates.md G2, rule 5), so the EVM deployer can be a Foundry keystore:
`script/deploy.sh`, `script/deploy-stack.sh` and `script/deploy-wrapclaim.sh` take `--account <name>` on 4663 beside `--ledger` /
`--trezor`. `script/deploy-nft.sh` does not: the NFT side is off in Phase 1.

1. **Make it on device A, never seeing the key:** `cast wallet new ~/.foundry/keystores rook-deployer` writes a new random key,
   encrypted with a password typed at a hidden prompt, and prints only its address. (`cast wallet import rook-deployer --interactive`
   is the alternative for an existing key.) Use a password used nowhere else. The key is on no other machine, in no backup, cloud
   drive, chat or screenshot; it only ever holds gas.
2. **Where it must be.** Forge's `--account` reads `~/.foundry/keystores/<name>` and nowhere else, whatever
   `FOUNDRY_KEYSTORES_DIR` says (measured 2026-09-25). The scripts check that same file, refuse when `FOUNDRY_KEYSTORES_DIR` names
   another directory, and refuse `ETH_KEYSTORE`, `ETH_KEYSTORE_ACCOUNT` and `ETH_PASSWORD` in the environment.
3. **Its address must be the deployer's.** The scripts compare the keystore's recorded address with the deployer they are given;
   if the file records none, `cast wallet address --account <name>` opens it once (the password is typed) and that is compared.
   The chain id is still typed by hand before forge starts, and forge asks for the password again when it signs.
4. **Fund it** with ≈ 0.03 ETH on 4663 (contracts runbook, "Before anything" 4), after checking with `cast code` that the address
   has no code (EIP-7702 sweepers, `docs/trust-model.md`).
5. **After the last deployment** send what is left back to the ops address and delete the keystore file on the public log. It owns
   nothing either way; deleting it removes the look-alike risk in the table below.

## The ceremony, in order

1. **Before the day.** Each Safe signer, the proposer and the guardian set up their own device on their own machine. Nobody else
   sees a seed. Each writes down their principal or address and sends only that.
2. **The Safe** is created on 4663 with the signers' addresses and its threshold; each signer confirms their address in the Safe's
   view on their own device.
3. **Principals, cross-checked.** Every proposer and every guardian reads out their principal (`dfx identity get-principal`) on a
   call; all are written into the timelock's install argument (`roles`) and read back from `status()` after install by someone who
   holds none of them.
4. **The operator's identity** is created on the loop's server and never leaves it.
5. **The IC deployer** does the whole of `docs/deploy/canister-mainnet.md` steps 1–7 in one sitting, then deletes its identity.
6. **The watcher** is started, and the guardian confirms they received its first message.
7. **A drill** within the first week: one guardian pauses the buyer and two vote to resume it; a proposer makes a harmless proposal
   (`propose_controllers` naming the same timelock), one guardian's veto is shown not to be enough and a second's vetoes it; and the
   watcher must report every step, and the buyer's and the timelock's cycles.
