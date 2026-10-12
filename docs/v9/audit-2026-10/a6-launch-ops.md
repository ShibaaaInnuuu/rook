# A6 — Solana launch operations (day 1, deploy-later, launch-params, fund-status, the Meteora configs)

| | |
|---|---|
| Baseline | `f2c24e0` on `golive-2026-10-08` (no source file modified by this audit) |
| Date | 2026-10-10 |
| Scope | `ops/solana/ceremony-day1.sh`, `ops/solana/deploy-later.sh`, `ops/solana/launch-params.mjs`, `ops/solana/fund-status.mjs`, `ops/rehearsal/sol-devnet/tool/src/ceremony.rs` (`rook-ceremony day1` / `init`) and what they call; the 12 collection DBC configs and the genesis ROOK config / pool / creator handover; the `listing_order` commitment; the reserved program-id swap; the deploy → init → handover sequence; the public explainer (`docs/v9/launch-day1-solana.md` §5) |
| Reference | Meteora DBC source at `f552f20` (Release 0.2.1, the deployed mainnet program), read independently for this audit |
| Added files | `ops/test/audit_a6_day1_verify.py` (independent decoder / verifier / read-only simulator), `ops/test/audit_a6_keys_path.sh` (PoC A6-01), `ops/test/audit_a6_launch_tamper.sh` (PoC A6-05) |
| Mainnet | read-only only: `simulateTransaction` (sigVerify off, nothing sent), `getMinimumBalanceForRentExemption`, `getBalance`, `fund-status.mjs`. No transaction was sent. |

## Summary

No Critical or High finding. The 13 day-1 transactions were decoded and simulated independently and are correct: every
`create_config` field matches decision D-e / D-g and passes DBC 0.2.1's validation; the claimer PDAs, the listing order and the
curve math agree with an independent implementation; the fund wallet signs nothing; the ROOK transaction hands the creator to the
fund wallet atomically. The id swap is complete (five places) and the swapped build carries both reserved ids and no dev id.
`init_registry` / `init_backing` are upgrade-authority gated, so the deploy → init window cannot be front-run.

The findings are about the irreversible edges: guards that do not hold (the keys-in-repository refusal, A6-01; the launch-file
checks, A6-06), commitments with no enforcing tool (the listing order, A6-03), and the deploy step building from an unpinned tree
and trusting whatever launch file / RPC it is given (A6-04, A6-05).

| ID | Title | Severity | PoC |
|---|---|---|---|
| A6-01 | The "keys inside the repository" refusal is bypassable (symlinked leaf, letter case on APFS); no `.gitignore` rule for keypairs | Medium | `ops/test/audit_a6_keys_path.sh` — 4 bypasses reproduced |
| A6-02 | The listing-order commitment is enforced by nothing; the runbook's safety claim (`WrongFeeClaimer`) does not hold when configs are paired by id | Medium | reasoning + `audit_a6_launch_tamper.sh` (reordered order accepted) |
| A6-03 | deploy-later builds and deploys the live working tree (unpinned, unverifiable); a rerun silently upgrades rook_core | Medium | reproduction below (untracked files land in the build copy) |
| A6-04 | deploy-later / `rook-ceremony init` never check the cluster against the launch file: an immutable wrong Bitcoin network, or ids that do not match the day-1 configs | Medium | static reproduction below |
| A6-05 | Permanent config fields, the listing order and the review flag are not pinned at run time | Medium | `ops/test/audit_a6_launch_tamper.sh` — 7/7 tampered files accepted |
| A6-06 | The runbook's `--send` line ships the ROOK coin with placeholder metadata (empty URI) on an immutable mint | Low | offline plan: `genesis_rook.uri = ""`; verifier flags it |
| A6-07 | Pre-funding a published program id blocks `solana program deploy` | Low | local validator reproduction below |
| A6-08 | Upgrade authority can stay on the hot ceremony key; handover target is unchecked beyond "funded system account" | Low | — |
| A6-09 | "Already exists" paths trust a single RPC and do not verify contents (day-1 configs partially; registry / settings not at all) | Low | — |
| A6-10 | Cost estimates and deploy mechanics: simulated cost uses an active exchange wallet; no priority fee for ≈ 1,360 deploy writes; the CLI prints a buffer seed phrase on failure | Low | local validator (seed phrase printed, redacted) |
| A6-11 | fund-status's target is the program deposit only; the post-deploy listing ceremony is unfunded | Low | — |
| A6-12 | Explainer omissions: "refundable" deposit, and what holders of pre-deploy coins depend on | Low | — |
| A6-13 | Off-chain components hard-code the dev program ids (canister `solread::ROOK_BACKING_ID`) | Informational | grep |
| A6-14 | Doc / source inaccuracies and small guards (burn claim, `localnet` genesis skip, NaN spread, decoy keypairs from `cargo build-sbf`) | Informational | — |
| A6-15 | Anyone can launch pools on the ROOK config or create look-alike configs paying Rook's claimers | Informational | — |

## Threat model

**Assets.** (1) The two reserved program keypairs: whoever holds them can deploy any code at the ids and so sign as every claimer
PDA (every graduated coin's 42.5 SOL migration fee, all partner curve fees, the partner LP positions), and losing them strands all
of it in DBC forever. (2) The fund wallet's SOL and its future roles (upgrade authority, registry owner). (3) The ceremony payer
(hot; ≈ 0.11 SOL on day 1, ≈ 9.5 SOL on deploy day, and the upgrade authority until handover). (4) The correctness of permanent
state: 13 DBC configs (no close, no update), the ROOK mint's immutable metadata, rook_core's registry (singleton, `owner` /
`bitcoin_network` fixed at init), listing ids 0–11 and their immutable fields.

**Actors.** Operator error (wrong file, wrong directory, wrong RPC, forgotten flag, rerun after a partial failure); a compromised
machine (out of reach of any script: noted where a script could narrow it); a malicious or faulty RPC answering the reads;
a front-runner who learns the reserved ids or the configs.

**Front-running questions, answered.**

| Question | Answer |
|---|---|
| Can someone deploy at our ids? | No: the program account is created with the program keypair's signature. They can only **block** the stock CLI by pre-funding the address (A6-07). |
| Do the day-1 configs reveal the ids? | No. Configs carry the claimer PDAs (SHA-256 outputs); the ids appear only if published or in the deploy transaction itself. |
| Can someone create a config / pool rook_backing will accept? | Configs: anyone can create a DBC config with `fee_claimer` = a Rook claimer (A6-15), but `register_partner_config` is admin-only (registry owner), so it is accepted only if the operator registers it. Pools: anyone can launch on Rook's registered configs — that is the design (`open_vault` accepts any pool on a registered config). |
| Can someone squat listing PDAs? | No: `["listing", u32]` accounts are created only by `list_collection` (registry owner, `registry.rs:243`). |
| Can someone `init_registry` / `init_backing` first? | No: both require the ProgramData upgrade authority (`registry.rs:56`, rook_backing §7.1). Between `solana program deploy` of rook_core and `init_registry` (minutes, while rook_backing's buffer is written) every other rook_core init needs the registry owner, so there is nothing to front-run. |
| What can a malicious RPC do? | Day 1: make the tool skip creating configs (fake "exists: checked") or misstate cost — DoS, no theft (configs need the config keypair's signature; messages are signed locally). Deploy: misreport balances / genesis; it cannot change what is signed. A6-09. |

## Findings

### A6-01 — The "keys inside the repository" refusal is bypassable; keypairs are not git-ignored — Medium

**Where.** `ops/solana/ceremony-day1.sh:37-40`, `ops/solana/deploy-later.sh:38-39`, `.gitignore`.

**Description.** ceremony-day1.sh resolves only the **parent** of `--keys` (`cd "$(dirname "$KEYS")" && pwd -P`) and appends the
unresolved basename, so a `--keys` that is itself a symlink into the repository passes (the comment says "symlinks resolved").
Both scripts compare paths as case-sensitive strings, but macOS volumes are case-insensitive by default and bash's `pwd -P`
keeps the case it was given. This machine uses both spellings: the session's shell sits in `/Users/jlohayon/Documents/rook`
while the repository is `/Users/jlohayon/Documents/Rook` (same inode 38933980). Nothing in `.gitignore` matches
`rook_core-keypair.json`, `rook_backing-keypair.json`, `ceremony-payer.json` or the config keypairs.

**Impact.** The program keypairs land in the working tree as untracked files; a routine `git add -A` / commit / push puts them
on GitHub (the repository is private today) and in every clone and CI cache. Whoever reads them can deploy at the reserved ids
before Rook and take every claimer's funds (A-asset 1).

**Reproduction.** `bash ops/test/audit_a6_keys_path.sh` (offline; creates and removes an empty directory):

```
BYPASSED ceremony-day1.sh --keys <symlink to a dir in the repo>
BYPASSED ceremony-day1.sh --keys /Users/jlohayon/Documents/Rook/ops/test/.audit_a6_tmp_…-case (same directory, other case)
BYPASSED deploy-later.sh --keys /Users/jlohayon/Documents/Rook/ops/test/.audit_a6_tmp_…-case (same directory, other case)
BYPASSED .gitignore has no rule for rook_core-keypair.json / ceremony-payer.json / *-config.json
4 bypass(es)        (exit 1; exit 0 once fixed)
```

**Recommendation.** Resolve the full path (`realpath` of the directory after `mkdir`, or `python3 -c 'os.path.realpath'`), and
decide "inside" by identity, not by string: refuse if `git -C "$KEYS" rev-parse --show-toplevel` succeeds (any repository), or
compare `stat -f %d:%i` of each ancestor with the repository root's. Add `*keypair*.json`, `ceremony-payer.json`, `day1/`
to `.gitignore` as a second layer.

### A6-02 — The listing-order commitment is enforced by nothing — Medium

**Where.** `ceremony.rs:607-615` (claimer from `listing_id` only), rook_core `registry.rs:232,260` (listing PDA = sequential
`listing_count`), `docs/v9/launch-day1-solana.md:36-40`, `:145-147` (the listing tooling "comes with the program review").

**Description.** Day 1 binds each collection's DBC config, permanently, to `claimer(listing(i))` for a listing **id** that
does not exist yet. rook_core gives ids in call order and every listing field that matters (`kind` with chain/collection or
membership root, `custodian`, `poster`, `custody`) is immutable (`state/registry.rs:102-127`); there is no delist or edit. So:

- a `list_collection` in the wrong order, or one with a wrong field (a typo in a contract address, the wrong canister key as
  `custodian`), burns that id for good, and the collection's config keeps paying that listing's claimer;
- the runbook says a wrong order is caught because `register_partner_config` "would then refuse the config (`WrongFeeClaimer`)".
  That holds only if the operator pairs configs with partners **by slug**. Paired **by listing id** (the natural loop: config i
  → partner of listing i) the check passes and, for example, every Milady coin's 42.5 SOL is converted into the Bitcoin Frogs
  floor module. The check verifies `config.fee_claimer == partner.claimer`, nothing about which collection the listing is;
- the tool that should prevent all this does not exist, and `rook-ceremony day1` itself accepts a reordered or truncated
  `listing_order` (A6-05).

**Impact.** Irreversible misdirection of a collection's backing, or a collection whose proceeds can only ever back a broken
listing. Funds are not stolen, but holders of that collection's coins are backed by the wrong NFTs or by nothing usable.

**Recommendation.** Write and review the listing tool **before day 1**, since day 1 is what makes the ids binding: for each `i`,
assert `registry.listing_count == i`, build `ListArgs` from `launch.listing_order[i]` (slug, kind, chain id, collection, root)
and the deployment record (custodian, poster, custody), simulate, send, then read the listing back and compare every field;
register partners/configs from `day1-plan.json` **by slug** and assert `partner.listing == listing(i)` and
`config.fee_claimer == claimer(listing(i))` both. If day 1 has not run yet, consider deriving the claimer from a collection
identity seed (e.g. `["claimer", chain_id, collection]`) so the order stops mattering. Correct the runbook sentence.

### A6-03 — deploy-later builds the live working tree; a rerun silently upgrades — Medium

**Where.** `ops/solana/deploy-later.sh:52` (`rsync -a --delete … "$ROOT/solana/"`), `:59-62`, `:101-104`.

**Description.** The deploy copies whatever is in `solana/` at the time — uncommitted edits and untracked files included — with
no check that the tree is clean or at the audited commit, no `--locked`, and no record of the commit or the `.so` hash. The audit
copy made the same way contained other auditors' untracked test files (`audit_a1_core.rs`, `audit_a2_core.rs`,
`audit_a3_backing.rs`); a stray source edit would be deployed the same way. The build is not a verifiable (`solana-verify`
docker) build, so nobody can show the deployed bytes match a published commit. On a rerun after a partial failure (rook_core
deployed, rook_backing not) `solana program deploy … --program-id rook_core-keypair.json` **upgrades** rook_core from the
then-current tree, needing a full new buffer (≈ 4.4 SOL transient), and the balance check still demands the whole peak.

**Impact.** Unreviewed code at the ids that control all backing, with no evidence trail; a non-idempotent recovery path that can
change deployed code without saying so.

**Reproduction.** `rsync -a --exclude target --exclude .anchor solana/ <dir>/solana/` (the script's line 52) on today's tree, then
`ls <dir>/solana/programs/*/tests | grep audit` lists the three untracked files. `grep -n "git\|sha256\|--locked" deploy-later.sh`
finds nothing.

**Recommendation.** Build from `git archive <signed tag>` (or refuse unless `git status --porcelain solana/` is empty and HEAD is
the tag named on the command line); use `cargo build-sbf --locked` or a `solana-verify build`; print and record the sha256 of each
`.so` and the commit; after deploying, `solana program dump` and compare hashes. On rerun, skip a program that is already deployed
with identical bytes and refuse one deployed with different bytes unless `--upgrade` is given.

### A6-04 — deploy-later / `rook-ceremony init` trust the launch file and keys directory against any cluster — Medium

**Where.** `deploy-later.sh:45-47` (reads `bitcoin_network`, prints the genesis hash, compares nothing), `:107-108`;
`ceremony.rs:732-787` (`init`: no cluster check; contrast `day1`, `ceremony.rs:572-582`); rook_core `registry.rs:66`.

**Description.** `init_registry`'s Bitcoin network is immutable and the registry is a singleton. deploy-later passes whatever
`bitcoin_network` the given launch file names and never compares the file's `cluster` with the RPC's genesis. The devnet
rehearsal writes launch files with `--bitcoin-network signet` (`ops/rehearsal/sol-devnet/run.sh:204`), so passing one of those on
deploy day sets `Signet` on mainnet for good: Bitcoin Frogs (listing 11) then needs a `tb1p…` custody address, and the canister
refuses a frog listing whose registry names another network. Only a program upgrade with a migration instruction could repair it.
Likewise deploy-later never checks that the keys directory is the day-1 one: it reads nothing from `day1-plan.json`, so a
regenerated directory deploys at ids whose claimers match none of the 13 live configs (the deposit is recoverable by `close`,
but the ids are burned).

**Reproduction (static).** `deploy-later.sh:45` reads `bitcoin_network`; `:46-47` print `GEN` and nothing compares it;
`rook-ceremony init` takes `--bitcoin-network` without a `cluster`. `day1` does refuse a mismatch (`ceremony.rs:580`).

**Recommendation.** In both: refuse unless `genesis == MAINNET_GENESIS ⇔ launch.cluster == "mainnet-beta"` and, on mainnet,
`bitcoin_network == "Mainnet"`. Before deploying, recompute the 12 claimers from the keypairs and require them to equal the
`fee_claimer` of the on-chain configs listed in `day1-plan.json` (read through two RPCs). After `init`, decode the registry and
assert owner, network and converters.

### A6-05 — Permanent config fields, the listing order and the review flag are not pinned at run time — Medium

**Where.** `ceremony.rs:225-262` (`check_params`), `:265-288` (`check_written`), `:550-553` (listing order), `:538-541`.

**Description.** D-e is stated as "`rook-ceremony` refuses a launch file that drifts from D-e", but the run-time checks cover only
part of it. Never checked against fixed values at run time (only in a unit test against the generator, not against the file
being used): `sqrt_start_price`, `curve`, `migration_fee_option`, `migrated_pool_fee`, `migrated_pool_base_fee_mode` and the
market-cap scheduler, `compounding_fee_bps`, `pool_creation_fee`, `token_update_authority`, `locked_vesting`. The simulated
account check (`check_written`) does not read them either. The `listing_order` is accepted if its ids are `0..n` — any length,
any order. `review.reviewed_by = null` (the generator's "fill in by hand after reading every line") is not required.

**Impact.** A hand edit during review, a stale or wrong file, or a tampered one produces 12 permanent configs that pass every
check and the simulation: e.g. creators allowed to rewrite metadata, a 100 SOL pool-creation fee that kills launches, a 10 % DAMM
pool fee, or the listing order changed (A6-02).

**Reproduction.** `bash ops/test/audit_a6_launch_tamper.sh` (offline):

```
ACCEPTED listing_order-3-entries      ACCEPTED listing_order-frogs-first    ACCEPTED migrated_pool_fee-10pct
ACCEPTED migration_fee_option-0       ACCEPTED pool_creation_fee-100SOL     ACCEPTED token_update_authority-0
ACCEPTED untampered-unreviewed        7 tampered/unreviewed launch file(s) accepted (exit 1)
```

**Recommendation.** Compile the complete D-e / D-g parameter sets into the tool and require byte equality of the Borsh-encoded
`ConfigParameters` (collection and ROOK) — or a SHA-256 of each committed in the repository and in the reviewed launch file. Check
`listing_order` against `deployments/launch/nft-pilot.json` (12 entries, Bitcoin Frogs last). Refuse `--send` while
`review.reviewed_by` is null. Extend `check_written` to the whole account (the independent verifier added here does).

### A6-06 — The runbook's `--send` ships ROOK with placeholder metadata on an immutable mint — Low

**Where.** `docs/v9/launch-day1-solana.md:83`; `launch-params.mjs:158-159` (name "Rook", symbol "ROOK", uri ""); `ceremony.rs:543-549`.

**Description.** The dry-run line lists `--rook-name/--rook-symbol/--rook-uri` as "placeholders until set"; the `--send` line
omits them. The tool accepts an empty URI and the placeholders, and ROOK's config has `token_update_authority = 1` (immutable
metadata), so the genesis coin is launched forever without an image or description. The confirmation prompt does not show the
metadata.

**Impact.** The fund-raising coin is launched in a state terminals show as a blank token; the only remedy is a second ROOK pool on
the same config (which then competes with the first for the same ticker).

**Reproduction.** The offline plan built for this audit (`rook-ceremony day1` without `--rook-*`) has `genesis_rook.uri = ""`;
`audit_a6_day1_verify.py` reports `FAIL ROOK metadata uri set`.

**Recommendation.** Refuse `--send` with an empty URI or the placeholder values unless an explicit `--rook-final` is given; fetch
the URI, check it is JSON whose name/symbol match and whose image resolves; print name/symbol/uri in the `Type 'day1'` prompt;
put the flags in the runbook's `--send` line.

### A6-07 — Pre-funding a published program id blocks `solana program deploy` — Low

**Where.** `deploy-later.sh:101-104`.

**Description.** Agave 4.1.2's `solana program deploy --program-id <keypair>` refuses when the program address already holds
lamports. Anyone who knows a reserved id can send it 0.001 SOL. The holder can recover (sign a transfer out of the address with
the program keypair, then deploy), but a watcher can re-fund between the two transactions. Day-1 configs do not reveal the ids;
publishing them (site, canister configuration, docs) does.

**Reproduction (local `solana-test-validator` 4.1.2).** Pre-funding a fresh program address with 0.001 SOL →
`Error: Account 4uKXjz… is not an upgradeable program or already in use`. After `solana transfer --from prog.json … ALL` the
same deploy passes that check.

**Recommendation.** Keep the ids unpublished until the deploy lands (publish the claimer PDAs only), and prepare a pre-funding-
tolerant final step: one transaction of `transfer` (top-up to rent) + `allocate(36)` + `assign(loader)` (signed by the program
keypair) + `DeployWithMaxDataLen`.

### A6-08 — Upgrade authority can stay on the hot ceremony key; unchecked handover target — Low

**Where.** `deploy-later.sh:111-119`.

**Description.** Without `--handover`, or if the run stops between deploy and handover (`init` fails under `set -e`), the
upgrade authority of both programs stays on `ceremony-payer.json` — a hot key in a directory that also holds a full source tree
and receives build scripts' output — with no reminder beyond one line. The handover uses `--skip-new-upgrade-authority-signer-check`;
the only guard on `--fund-wallet` is "a funded system account", which any mistyped live address satisfies.

**Recommendation.** Make the handover part of `--send` (opt out explicitly), and on any failure after the first deploy print the
recovery command prominently. Prefer the checked variant (one signature by the fund wallet), or require the operator to type the
last 6 characters of the new authority. Parse `solana program show` and assert the new authority.

### A6-09 — "Already exists" paths trust one RPC and verify little — Low

**Where.** `ceremony.rs:660-663` (existing config: `check_written` subset), `:767-770` (`init`: registry / settings existence only).

**Description.** A config that exists is "checked" only on the `check_written` subset (A6-05) and through the same single RPC
that answers everything; a lying RPC can make the tool skip creating configs and print "EXISTS (checked)". `init` skips
`init_registry` / `init_backing` if the account exists at all, without reading owner / network / converters, and never reads them
back after sending.

**Recommendation.** For every "exists" and after every send, read through two independent RPCs and require equal bytes; compare the
whole config account to the expected one; decode the registry and settings.

### A6-10 — Cost estimate and deploy mechanics — Low

- `simulate_step` measures cost as `getBalance(sim payer)` minus the simulated post-balance (`ceremony.rs:510-525`). The stand-in
  payer is an exchange hot wallet whose balance moves every slot, so the "exact SOL" can be off in either direction (saturating at
  0). An under-funded payer leaves a partial day 1 (recoverable by rerun). Compute rent + 5,000 × signatures instead; the measured
  2026-10-09 figure happens to equal that arithmetic (12 × 0.005984080 + 0.026575720).
- deploy-later sets no `--with-compute-unit-price` for ≈ 1,360 write transactions on mainnet and budgets no priority fee.
- On a failed deploy the CLI prints the buffer's 12-word seed phrase (reproduced locally; redacted). The buffer authority is the
  payer, so the phrase alone cannot drain it, but the runbook's "never prints a secret" is not true of this step; keep the log private.

### A6-11 — fund-status's target is only the program deposit — Low

**Where.** `fund-status.mjs:39-48`.

**Description.** The target is deploy peak + write fees + 0.06 SOL. After deploy, the registry owner (the same wallet) must still
pay rent for `init_treasury`, `init_desk`, 12 × `list_collection`, modules, wrappers / wX mints, 12 × `register_partner`,
`register_partner_config`, and the canister's own Solana costs; none is in the target, so "100 %" does not mean "launchable". The
"raised" side was checked and does not overstate (wallet balance + `creator_quote_fee` @360 + `partner_quote_fee` @272 of DBC
0.2.1's `VirtualPool`, offsets verified from source); it also counts any SOL the operator holds in the wallet for other purposes.
One RPC, no cross-check; program sizes are defaults unless passed.

**Recommendation.** Add the listing-ceremony rent (sizes × the cluster's rent) as a second line, or label the line "deploy
deposit" only; read through two RPCs before publishing.

### A6-12 — Explainer omissions — Low

**Where.** `docs/v9/launch-day1-solana.md:167-180`.

The explainer correctly states contributions are donations with no return, ROOK has no backing or rights, and the fund wallet is a
single point of failure. It does not say: (a) the "refundable" deposit is recoverable only by **closing** the programs, which
permanently ends every claimer's ability to sign — all pending and future backing becomes unclaimable — and the refund goes to the
upgrade authority, not contributors; (b) until the deploy, every collection coin's backing depends on the operator keeping two
program keypairs safe and on the fund reaching its target; if the deploy never happens, 42.5 SOL per graduated coin stays in DBC
forever. Recommend stating both, and that the programs' code at deploy time is chosen by the key holder.

### A6-13 — Off-chain components hard-code the dev ids — Informational

`autobuyer/canister-rs/src/solread.rs:634` (`ROOK_BACKING_ID`, used for partner PDAs and vault listing, and by `ethflow.rs:6594`)
and `solix.rs:13` name the dev ids; rook_core's id is configurable through `Home::Solana`, rook_backing's is not. deploy-later's
swap covers only `solana/`. Before go-live the canister must be rebuilt with the reserved rook_backing id (cross-reference A5).

### A6-14 — Doc / source inaccuracies and small guards — Informational

- `launch-params.mjs:90`: "DBC burns the unsold curve coins at migration". With pre = post supply DBC's maximum burnable is
  `pre − post = 0` (`state/config.rs:931`): nothing is burned. The tail of the curve (12.775167 coins per collection coin,
  9.031487 for ROOK — computed independently and read back from the simulated configs) plus any unsold remainder goes to the
  leftover receiver (the claimer PDA, for which rook_backing has no instruction yet). "Supply stays 10^15" is true.
- `ceremony.rs:575-579`: a `localnet` launch file skips the genesis check (harmless for day 1: the configs do not depend on it).
- `launch-params.mjs:362`: `--max-spread-bps abc` gives `NaN` and disables the spread refusal; no ticker staleness check (Kraken
  SOLETH is thin). The day-1 configs do not use the rate.
- `cargo build-sbf --sbf-out-dir "$B/deploy"` writes fresh random `rook_core-keypair.json` / `rook_backing-keypair.json` next to
  the `.so` files (reproduced). deploy-later passes `$KEYS/<p>-keypair.json` explicitly, so it is correct, but a manual recovery
  from `$B/deploy` would deploy at the wrong id. Delete them after the build.

### A6-15 — Anyone can launch on the ROOK config or create look-alike configs — Informational

DBC pool creation is permissionless, so others can launch "ROOK" lookalikes on the genesis config: same config, same partner (the
fund wallet), creator share 100 % to **their** creator; the fund earns nothing from them, and terminals show them under Rook's
config. Anyone can also create configs with `fee_claimer` = a Rook claimer and other parameters; `register_partner_config` checks
only a subset (A3), so registration must take config addresses from `day1-plan.json`, never discover them by `fee_claimer`
(nothing in the repository does so today). Publish the official ROOK mint / pool and the 12 config addresses.

## Checked and found sound

**Independently decoded day-1 transactions** (`ops/test/audit_a6_day1_verify.py`: own base58, ed25519 on-curve test, PDA
derivation, legacy-message parser, Borsh layout of `ConfigParameters` from DBC's `process_create_config.rs:40-71` and
`fee_parameters.rs`, Anchor discriminators from `sha256("global:…")`). Plan built offline with throwaway reserved ids; 150 checks
pass, 1 fails (A6-06, the placeholder URI).

| Field | Collections (12, byte-identical) | ROOK | Intent / DBC 0.2.1 bound |
|---|---|---|---|
| accounts | config, fee_claimer = leftover_receiver = claimer PDA, wSOL, payer, system, event authority, DBC | fee_claimer = leftover = fund wallet | D-e / D-g |
| base fee | cliff 500,000,000 (50 %), 60 periods × 10 s, −8,166,666/period, linear (mode 0) → 1.000004 % at 10 min | same | ≥ 0.25 %, ≤ 99 % |
| dynamic fee | none | none | |
| collect / migration option / activation / token type / decimals | 0 (quote) / 1 (DAMM v2) / 1 (timestamp) / 0 (SPL) / 6 | same | rook_backing requires the first four |
| LP | partner 0 + 50 locked, creator 0 + 50 locked, no vesting | same | sum 100, ≥ 10 % locked at day 1 |
| threshold | 85,000,000,000 lamports | same | |
| migration fee | 50 % / creator 0 | 0 / 0 | ≤ 99; creator 0 when fee 0 |
| creator trading fee | 0 % | 100 % (= 80 % of each curve fee after the 20 % protocol share) | ≤ 100 |
| supply | fixed, pre = post = 10^15 | same | |
| locked vesting / pool creation fee / first-swap min fee / padding | none / 0 / false / 0 | same | |
| migrated pool | option 6 (Customizable), fee 0 / dynamic 0 / 100 bps, base mode 0, scheduler 0, compounding 0 | same | 10–1000 bps |
| token_update_authority | 1 (immutable) | 1 | mint-authority variants refused |
| sqrt_start_price / curve | 134452599321738966; (268905193275807567, 2.151e32), (MAX, 3.435e24) | 95072344172433750; (380289371323205464, 1.014e32), (MAX, 3.435e24) | |
| signers | payer + config only; fund wallet absent | payer (= creator), config, mint; fund wallet read-only, non-signer | fund wallet signs nothing |

**Curve math, recomputed with DBC's integer formulas** (`liquidity_distribution.rs`, `curve.rs`, `concentrated_liquidity.rs`):
collections — start 5.3125e-8 SOL/coin (mcap 53.13 SOL), graduation 2.1250e-7 (mcap 212.50 SOL), 799,999,984.031 coins sold on the
curve, 200,000,003.194 to DAMM v2 with 42.5 SOL, migration fee exactly 42.5 SOL; the DAMM opening price equals the curve's end price
(no gap); fixed-supply validation passes with buffer + pool = 1,000,000,000.000 coins (exactly at the limit). ROOK — start
2.6563e-8 (26.56 SOL), graduation 4.25e-7 (425.00 SOL), 85 SOL to the pool, 0 migration fee, validation passes. These equal the
docs' figures and, below, the values DBC itself wrote in simulation.

**Read-only mainnet simulation** (2026-10-10, `api.mainnet-beta.solana.com`, the plan's unsigned messages with the fee payer
swapped for a funded account, sigVerify off): **13 / 13 succeed**. The written `PoolConfig` accounts, decoded at offsets taken from
DBC's `state/config.rs` (1,048 bytes, discriminator `account:PoolConfig`), carry exactly the fields above, `swap_base_amount` and
`migration_base_threshold` equal to the independent computation, and the ROOK `VirtualPool` shows `creator = GGgXnNgt…6Wuy`,
`config` and `base_mint` as planned after `transfer_pool_creator` in the same transaction. Config rent 5,974,080 lamports
(1,048 bytes) matches the runbook.

**PDAs and order.** For all 12 listings `listing = PDA(["listing", u32 LE i], rook_core)` and
`claimer = PDA(["claimer", listing], rook_backing)` recomputed; every claimer is off-curve; `converter = PDA(["converter"],
rook_backing)` and `registry = PDA(["registry"], rook_core)` recomputed. Order = `nft-pilot.json` file order (milady, azuki, mayc,
pudgypenguins, bayc, doodles, moonbirds, clonex, lilpudgys, checks, identitymd), Bitcoin Frogs last (id 11). Seeds match the
programs (`seeds.rs:9`, `constants.rs:23,27`; `listing_count: u32`).

**Program-id swap.** The five places (`rook_core/src/lib.rs` `declare_id!`, `rook_core/src/state/wake.rs` `ROOK_BACKING_ID`,
`rook_backing/src/lib.rs` `declare_id!`, `rook_backing/src/constants.rs` `ROOK_CORE_ID`, `Anchor.toml`) are the only ones under
`solana/programs` (the remaining occurrence is `solana/tests/mock_rook_core`, not deployed). A swapped copy built with
`--arch v3`, opt-level 3 gives 858,296 / 502,896 bytes (= the runbook's table), e_flags 3, each binary containing **both**
reserved ids and **no** dev id — no bricked deploy from a missed place. `overflow-checks = true`, `lto = "fat"` in the release
profile; opt-level 3 is the audited and rehearsed level.

**Other items checked.**
- `set -euo pipefail` in both scripts; arguments quoted; `--send` requires `--rpc` and a typed confirmation (`day1`, `deploy`,
  `handover`); keypairs generated with umask 077 / mode 0600, directory 0700, never overwritten (`solana-keygen` without `--force`;
  `config_kp` uses `create_new`), never printed; the plan holds no secret.
- Day-1 rerun safety: config / mint keypairs are reused, an existing config is skipped (never recreated), each transaction is atomic
  (the ROOK config + pool + creator handover cannot half-land), `send_one` resends identical bytes until the blockhash expires so at
  most one copy lands.
- The ceremony payer is refused if equal to the fund wallet; the fund wallet's only day-1 action is one plain transfer.
- `init_registry` and `init_backing` are upgrade-authority gated and the tool checks the authority before sending; registry
  owner = fund wallet, Bitcoin network from the file, initial converter = rook_backing's `["converter"]` PDA.
- deploy-later's rent arithmetic (program 36, ProgramData 45 + n, buffer 37 + n bytes; rook_core first is the lower peak) and the
  fund-status target (9.54068916 SOL today, 5,080 lamports/byte) are correct for loader v3.
- launch-params: ≥ 2 sources, refusal above 1 %, lower-median, no float past parsing, never overwrites a file without `--force`,
  `mainnet-beta` forces Bitcoin `Mainnet`; the day-1 configs do not depend on the rate.
- The explainer promises no return and discloses ROOK as an unbacked memecoin and the single-key risk (omissions in A6-12).
- fund-status reads the right `VirtualPool` offsets (`partner_quote_fee` 272, `quote_reserve` 240, `is_migrated` 305,
  `creator_quote_fee` 360) and does not overstate raised funds.

## Test results

| Test | Result |
|---|---|
| `cargo test --release --bin rook-ceremony` (existing) | 4 passed |
| `python3 -I ops/test/audit_a6_day1_verify.py --plan … --launch …` (offline decode) | 150 ok, 1 FAIL (A6-06 placeholder URI) |
| same with `--rpc https://api.mainnet-beta.solana.com --simulate` (read-only) | 13 / 13 simulations succeed; all written-account checks ok |
| `bash ops/test/audit_a6_keys_path.sh` | exit 1: 4 bypasses (A6-01) — exits 0 once fixed |
| `bash ops/test/audit_a6_launch_tamper.sh` | exit 1: 7 / 7 tampered or unreviewed launch files accepted (A6-05, A6-02) — exits 0 once fixed |
| swapped-id SBPF v3 build (scratch copy) | both binaries carry both reserved ids, no dev id |
| `solana-test-validator` 4.1.2: pre-funded program id | `solana program deploy` refused; passes after draining (A6-07); failed deploy prints a buffer seed phrase (A6-10) |

Note: running `ceremony-day1.sh` itself against mainnet (even without `--send`) was blocked by this session's permission policy;
the plan was built with `rook-ceremony day1` offline and simulated by the independent verifier instead.

## Verification (independent, 2026-10-10)

Method: I re-read both scripts and `ceremony.rs` at `f2c24e0` and ran:

- `bash ops/test/audit_a6_keys_path.sh`: 4 bypasses reproduced.
- `bash ops/test/audit_a6_launch_tamper.sh` (offline): 7 of 7 tampered or unreviewed files accepted.
- A fresh offline plan with throwaway ids (`rook-ceremony day1` with no `--rpc`), passed to `audit_a6_day1_verify.py`: only `FAIL ROOK metadata uri set`.

No mainnet call was made in this verification. I did not re-run the read-only mainnet simulation or the local-validator
reproductions (A6-07, A6-10).

Day 1 has **not** run on this machine (no `~/rook-keys`, no `~/rook-launch`). Every recommendation that changes the day-1 inputs,
including A6-02's identity-seeded claimer, is therefore still open.

| ID | Verdict | Final severity | Reason |
|---|---|---|---|
| A6-01 | CONFIRMED | Medium | `ceremony-day1.sh:37-39` resolves only the parent, and both scripts compare paths as case-sensitive strings. This shell's cwd really is `/Users/jlohayon/Documents/rook` while the repository is `…/Rook`. `.gitignore` has no keypair rule and there is no global excludes file. The runbook's `~/rook-keys` is safe, so it takes an operator deviation, but the asset is every claimer and a careless `git add -A` would stage the keypairs. |
| A6-02 | CONFIRMED | Medium | The claimer derives from `listing_id` only (`ceremony.rs:607-615`). No listing tool exists. The runbook's `WrongFeeClaimer` sentence (`launch-day1-solana.md:36-40`) holds only when pairing is by slug. Conversions made into the wrong module cannot be undone. |
| A6-03 | CONFIRMED | Medium | `deploy-later.sh:52` rsyncs the live `solana/` tree. There is no clean-tree or commit check, no hash record and no `--locked`, and a rerun upgrades through the payer's authority. |
| A6-04 | CONFIRMED | Medium | `deploy-later.sh:45-47` prints the genesis hash but compares nothing, and `init` takes `--bitcoin-network` blind. A wrong value is recoverable only through a program upgrade with a migration (possible under D-g, but not an admin operation). |
| A6-05 | CONFIRMED | Medium | `check_params` / `check_written` (`ceremony.rs:225-288`) omit the listed fields, including `migrated_pool_fee.collect_fee_mode` (A3-02's stranding case). `listing_order` is checked only for `0..n`. `review.reviewed_by` is never read. |
| A6-06 | CONFIRMED | Low | `ceremony.rs:546` accepts `uri.len() == 0`. |
| A6-07 | CONFIRMED (not re-run) | Low | This is consistent with the Agave CLI's behaviour. The ids are not revealed by the configs. |
| A6-08 | CONFIRMED | Low | `--skip-new-upgrade-authority-signer-check`, and handover is opt-in (`deploy-later.sh:111-119`). |
| A6-09 | CONFIRMED | Low | Existence checks run through a single RPC. |
| A6-10 | CONFIRMED (partly not re-run) | Low | No `--with-compute-unit-price` in the script. The seed-phrase output was not re-run. |
| A6-11 | CONFIRMED | Low | |
| A6-12 | CONFIRMED | Low | |
| A6-13 | CONFIRMED | Informational | `solread.rs:634` and `solix.rs:13` hold the dev ids. This must be tracked in A5 before go-live. |
| A6-14 | CONFIRMED | Informational | `launch-params.mjs:90` comment is wrong (pre = post, nothing is burned). `Number("abc")` is `NaN`. |
| A6-15 | CONFIRMED | Informational | |

No finding disputed or adjusted. One addition, recorded as a cross-reference rather than a new finding: A6-05 and A3-02 together
mean **no layer** (generator run-time checks, simulation check, or `register_partner_config`) enforces the migrated DAMM
`collect_fee_mode == 0`. Fixing A6-05 by byte-equality of the Borsh `ConfigParameters` closes this.

## Fixes (2026-10-10)

Scope of the fix: `ops/solana/*`, `ops/rehearsal/sol-devnet/tool` (`rook-ceremony`: `src/ceremony.rs`, new `src/ceremony_checks.rs`,
`src/ceremony_list.rs`; `rook-devnet` adapted to the programs commit `6588b81`), `docs/v9/launch-day1-solana.md`, `.gitignore`. No
program source was changed here (the program fixes are `6588b81`). Nothing was sent to mainnet: mainnet was not contacted at all
except the read-only clone of DBC / Metaplex into a local validator and one read-only `solana program show` of DBC.

| ID | Status | Fix | Where | Test |
|---|---|---|---|---|
| A6-01 | **Fixed** | One guard for both scripts: the whole `--keys` path resolved (`os.path.realpath`, symlinked leaf included), refused if any existing ancestor is the repository root **by inode** (so `…/rook` = `…/Rook` on APFS), if it compares inside the repository case-insensitively, or if `git rev-parse` succeeds from the nearest existing ancestor (any git work tree; `GIT_DIR`/`GIT_WORK_TREE`/`GIT_CEILING_DIRECTORIES` cleared). ceremony-day1.sh checks again after `mkdir`. `.gitignore`: `*keypair*.json`, `ceremony-payer.json`, `day1/`, `day1-plan.json`. | `ops/solana/keys-guard.sh`; `ceremony-day1.sh:49,52`; `deploy-later.sh:56`; `.gitignore:26-32` | `bash ops/test/audit_a6_keys_path.sh` → **0 bypasses, exit 0** (4/4 held); another repo, `.git/`, and an outside path checked by hand |
| A6-02 | **Fixed** | `rook-ceremony list`: preflight first; lists strictly from `listing_order` (itself pinned to nft-pilot.json); asserts `Registry.listing_count == i` before each `list_collection` and `== i+1` after; reads every listing field back (id, kind with chain / collection or root, name, custody, custodian, poster) and stops on any difference; then `init_wrapper` (wX metadata from nft-pilot.json), `init_module` (launch file params), `add_collection`, `register_partner` (partner read back: listing, registry, module, vault, wrapper, wX mint, claimer = `claimer(listing(i))`) and `register_partner_config` with the config **day1-plan.json names for that slug** (never discovered by `fee_claimer`, A6-15), after checking the on-chain config pays that claimer. Runbook sentence about `WrongFeeClaimer` corrected and the tool documented. | `ceremony_list.rs:247` (`list`), `:348,354` (count), `:169` (`listing_diff`), `:421` (config by slug); `launch-day1-solana.md` §2, §4 | unit `list_args_and_listing_diff`, `col_pdas_are_the_programs`; **localnet end to end** (below) |
| A6-03 | **Fixed** | deploy-later builds from `git archive <commit>` of `--ref` (without `--ref`: HEAD, refused unless `git status --porcelain -- solana` is empty, untracked included), `cargo build-sbf --arch v3 -- --locked`, records commit + each `.so` sha256 in `<keys>/build/<commit>/deploy-record.json`, compares `solana program dump` with the `.so` after each deploy; on a rerun a program deployed with the same bytes is skipped and one with other bytes is refused unless `--upgrade`. | `deploy-later.sh:86-120` (archive, build, record), `:127-145` (rerun), `:198` (dump check) | localnet: deploy recorded and dump-verified; rerun at HEAD → both "skipped" (identical bytes: the build is reproducible); rerun with `--opt-level s` → **"refused: rook_core would be UPGRADED"** |
| A6-04 | **Fixed** | `rook-ceremony preflight` (run by deploy-later before building, and by `init` / `list`): genesis ⇔ `cluster` (mainnet-beta ⇒ Bitcoin `Mainnet`; localnet refused on a public genesis), the launch file pinned + reviewed, `day1-plan.json` names these ids / payer / fund wallet / cluster / genesis, the 12 listing + claimer PDAs recomputed from the keys and every day-1 config **on chain** checked (`fee_claimer = leftover_receiver = claimer`, whole account), the ROOK config paying the fund wallet; optional `--rpc2` requires byte-equal reads. `init` takes the network from the launch file (`--bitcoin-network` must agree), decodes an existing registry (owner, network, converter) / settings (pinned registry, treasury, converter) instead of skipping on existence, and reads both back after sending. | `ceremony.rs:785` (`preflight`), `:907` (`init`), `:885` (`check_registry`); `ceremony_checks.rs:227` (`check_cluster`) | unit `the_cluster_must_be_the_launch_files`; localnet preflight + init (registry and settings read back and checked) |
| A6-05 (+ A3-02 cross-ref) | **Fixed** | The complete D-e and D-g `ConfigParameters` are compared by **byte equality of their Borsh encoding** with `ops/solana/day1-pins.json` (hex + sha256 per config: collection `8db45a9b…53ae1c`, ROOK `c50f8c33…a5fe96`; the file's own hashes are verified). This pins every field incl. curve, sqrt start price, `migration_fee_option`, migrated pool fee with `collect_fee_mode == 0`, `pool_creation_fee == 0`, `token_update_authority`, vesting, LP split; the accounts (`fee_claimer = leftover_receiver`) are built by the tool. `listing_order` must equal the pinned order **and** the order derived from the compiled-in nft-pilot.json (12, wMILADY … wBFROG after `0c5d4b9`, Frogs last). `--send` refused while `review.reviewed_by` is null. The written-account check now covers the whole `PoolConfig` at DBC 0.2.1 offsets (vesting bytes, version, migrated collect mode @360, dynamic fee, pool fee bps, creation fee @368, update authority, supplies). Pins regenerate only with `rook-ceremony pins --write`. | `ceremony_checks.rs:122,145,166,249,284`; `ceremony.rs:556,573,581,299,367` | `bash ops/test/audit_a6_launch_tamper.sh` → **0 accepted, exit 0** (extended to 17 tampered / unreviewed / bad-uri files + a control that must be accepted); units `the_committed_pins_are_the_generators`, `a_drifted_field_is_refused_by_the_pin`, `the_written_config_is_checked_byte_by_byte`; on localnet the real DBC-written accounts pass the byte check |
| A6-06 | **Fixed** | `--send` refuses an empty name / symbol / uri and any uri that is not a permanent content address (`ipfs://`, `https://ipfs.io/ipfs/`, `https://gateway.pinata.cloud/ipfs/`, `https://arweave.net/`, `https://res.cloudinary.com/`); on mainnet it fetches the JSON and requires name / symbol to match and the image to resolve as `image/*`. Operator decisions 2026-10-10: name **ROOK**, symbol **ROOK**, uri `https://res.cloudinary.com/ddwebzwrk/raw/upload/v1791607907/rook-token/rook.json` (80 bytes ≤ Metaplex 200; ROOK tx 1,165 bytes ≤ 1,232) are the defaults in the launch file, ceremony-day1.sh and the tool. The prompt prints the metadata; the runbook's `--send` line carries `--rook-*` and the never-delete warning. | `ceremony_checks.rs:181,199`; `ceremony.rs:581-589`; `ceremony-day1.sh:29,82`; `launch-params.mjs` genesis_rook.metadata | units `the_rook_uri_is_a_permanent_content_address`, `the_hosted_rook_metadata_resolves` (network, `--ignored`: passes against the hosted files); tamper cases `SEND-rook-uri-empty`, `SEND-rook-uri-placeholder` |
| A6-07 | Open (doc only) | Not changed: keep the ids unpublished until the deploy lands. A pre-funding-tolerant deploy step is not written. | — | — |
| A6-08 | **Fixed** | The handover is part of `--send` (`--keep-authority` to opt out, which prints the recovery command); the target must equal the launch file's `genesis_rook.fund_wallet` and `day1-plan.json`'s `fund_wallet`; the prompt shows its last 6 characters; `solana program show --output json` must then name it. A trap prints the recovery commands on any failure after the first deploy. | `deploy-later.sh:68-70` (target), `:178-185` (recovery trap), `:203-225` (handover + read-back) | localnet: both authorities read back = the fund wallet |
| A6-09 | Partly fixed | `--rpc2`: every account read by preflight / init / list must be byte-equal through both providers; existing configs are checked on the whole account; registry / settings decoded. Day 1's own "exists" path still uses one RPC unless rerun through preflight. | `ceremony.rs:768` (`account2`) | — |
| A6-10 | **Fixed** (2 of 3) | `--priority-fee` → `--with-compute-unit-price`, budgeted at 200,000 CU per write tx in the printed SOL; deploys use a buffer keypair file in the job dir (resumable; the CLI prints no phrase) and any 12-word line is redacted. The simulated-cost-via-exchange-wallet item is unchanged (a funded ceremony payer is now detected at `confirmed`). | `deploy-later.sh:58` (flag), `:155-162` (budget), `:187-197` (redaction, buffer keypair, deploy) | localnet deploy with `--priority-fee 1000` |
| A6-11 | **Fixed** | fund-status adds the listing ceremony (treasury, desk, 12 × listing / wrapper / wX mint / Metaplex metadata / item set / module / vault / module wX / partner, + 0.01 SOL Metaplex fee each, + signatures) as a second line and `launch_line`; sizes pinned to the programs by a Rust test. Program sizes updated to the `6588b81` build (889,744 / 519,288 B → target ≈ 9.87 SOL + ≈ 0.66 SOL). | `fund-status.mjs:24,59,101` | `fund_status_listing_ceremony_sizes_are_the_programs`; npm `audit A6 (2026-10-10)` |
| A6-12 | **Fixed** | Explainer: the long-term deposit comes back only by closing the programs (ends every claim; goes to the upgrade authority, not contributors); backing depends on the operator's key custody and the fund until deploy; the deployed code is the key holder's choice. | `launch-day1-solana.md` §4, §5 | — |
| A6-13 | Out of scope here | The canister's dev-id hard-coding is the canister's (commit `52cf5ab`: "rook_backing id from Init.home"). | — | — |
| A6-14 | **Fixed** | "burns the unsold coins" corrected (nothing burned; tail + remainder to the leftover receiver) in launch-params.mjs and the runbook; `localnet` launch files are refused on a public genesis; `--max-spread-bps` must be an integer 0..500 (NaN refused); the random keypairs `cargo build-sbf` writes next to the `.so` are deleted. | `launch-params.mjs:90,365`; `ceremony_checks.rs:227`; `deploy-later.sh:105` | units; npm test |
| A6-15 | **Fixed** (process) | Registration takes config addresses from `day1-plan.json` by slug only (A6-02); official addresses to be published by the operator. | `ceremony_list.rs:421` | — |

**Program interface changes adopted** (programs `6588b81`): `init_wrapper(meta)` + Metaplex accounts and `register_partner`'s four
rook_core accounts in both `rook-ceremony list` and `rook-devnet`; the devnet drill waits until a drand round is in the past before
`prove_round`. The D-e pinned config satisfies the new `register_partner_config` checks (leftover = claimer, locked LP, no vesting,
SOL-only migrated fees, no creation fee, version 0): all 12 registered on the local validator.

**Test results (2026-10-10).**

| Test | Result |
|---|---|
| `cargo test --release --bin rook-ceremony` | 13 passed, 1 ignored (network; passes with `--ignored`) |
| `cargo test --release --bin rook-devnet` | 3 passed |
| ops `npm test` | 120 passed (119 + 1 new) |
| `bash ops/test/audit_a6_keys_path.sh` | exit 0, 0 bypasses |
| `bash ops/test/audit_a6_launch_tamper.sh` | exit 0, 0 of 17 accepted, control accepted |
| offline day 1 (throwaway ids, no RPC) + `audit_a6_day1_verify.py` | 151 ok, 0 failures (default ROOK uri) |
| localnet end to end (`solana-test-validator` 4.1.2, DBC + Metaplex cloned read-only from mainnet, programs `6588b81`) | day 1 `--send` (13 tx; the real DBC-written configs pass the byte check); deploy-later `--send` (build from a pinned commit, dump-verified, init read back, handover read back); `rook-ceremony list --send`: **74 / 74 transactions, every account read back**, all 12 configs registered under the new on-chain checks; reruns: list → 74 steps "on chain (checked)", deploy-later → skipped / already done; a changed custody address → "listing 0 exists with other fields"; an `--opt-level s` build → refused as an upgrade |

**Left open.** A6-07 (pre-funding-tolerant deploy); A6-09 for day 1's own existence path; A6-10's cost-by-balance-delta; the
listing tool signs only with a keypair file (no Ledger); rook_core's size differs from the coordinator's 901,880 B (this build:
889,744 B, `cargo build-sbf --arch v3`, O3, reserved ids, reproducible) — `deploy-later.sh` prints the exact size it deploys.
