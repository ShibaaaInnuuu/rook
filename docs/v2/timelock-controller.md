# The buyer's controller: a public delay, a veto, and a build anyone can reproduce

Written 2026-09-23, before its code. Task #24; audit `docs/security/audit-btc-v1.md`, B1's closing note and "Decentralization".

## The problem, in the audit's words

> The trust anchor is now the controller set, and a controller can replace the wasm with one that signs anything. So "a key
> nobody holds" is exactly as true as "nobody controls the canister", and today one dfx identity does. … Until it is [built],
> the honest sentence for the site is *the operator can change what the buyer does, with a delay of zero*.

`docs/v2/autonomous-buyer.md` already chose the answer — "the only controller is a small, public timelock canister; an
upgrade is proposed with its Wasm hash, waits N days in public, and can be vetoed by a guardian key whose only power is veto
… (reproducible build required). A kill switch is 'stop pulling' — never 'move funds'." This document is how.

## What was read

- **The IC interface specification** (`dfinity/portal`, `docs/references/ic-interface-spec.md`): `install_code` and
  `update_settings` "can be called by canisters as well as by external users"; "only controllers of the canister can install
  code"; an upgrade is **atomic** ("if the response to this request is a `reject`, then this call had no effect");
  `/canister/<id>/module_hash` and `/canister/<id>/controllers` are readable by anyone through `read_state`, certified; the
  module hash is the SHA-256 of the module as installed; a gzip module is accepted and its hash is of the bytes uploaded.
- **Resource limits** (`docs/building-apps/canister-management/resource-limits.mdx`): 2 MiB maximum ingress payload, 2 MiB
  cross-subnet inter-canister payload. The buyer's wasm is 1.61 MB raw, 0.39 MB gzipped: one message carries it.
- **Reproducible builds** (`docs/building-apps/best-practices/reproducible-builds.mdx`): anyone reads a canister's module
  hash with `dfx canister --network ic info <id>`; a build is only checkable if the author supplies source, the build
  environment and a deterministic build; Docker is the recommended environment; for Rust, remap absolute paths
  (`--remap-path-prefix`) and pin the platform, because Linux and macOS builds can differ.
- **ic-blackhole** (`ninegua/ic-blackhole`): a canister whose only controller is a ~30-line canister with no install method is
  immutable; its authors verify it by three matching hashes — GitHub's build, a local build, and the deployed module.
- **Orbit** (`dfinity/orbit`): DFINITY's multi-custody "Station" plus an "Upgrader", with approval policies over requests,
  including upgrades of external canisters. It is a platform — user management, a wallet, a control panel, policies.

## The choice: a minimal timelock canister, and not the alternatives

| Option | Why not, here |
| --- | --- |
| Keep a person as controller | What the audit says is wrong. |
| Blackhole the buyer | Nobody can ever fix it. A PSBT parser with a bug stays a PSBT parser with a bug, and the buyer holds keys to money. |
| SNS | A DAO and a token for one canister and one operator; governance is not the problem being solved. |
| Orbit | It does multi-approval well, but a delay-plus-veto is a small fraction of what it is, and the thing a holder must read to trust the controller grows from one small canister to a platform. |
| **A minimal timelock** | ~300 lines a reviewer can read in one sitting; it does one thing; once installed it is itself blackholed. |

## What the timelock is

A canister, `timelock`, installed with four fixed facts and no way to change them:

- `target` — the buyer. The only canister it acts on.
- `proposer` — the operator's principal. The only one who may propose.
- `guardian` — the veto key. The only other principal with any power.
- `delay_secs` — the public delay. **At least 72 hours** in a production build; a build with the `rehearsal` cargo feature
  lowers the floor to 10 seconds for replica tests, and that feature changes the module hash, so a rehearsal timelock can never
  pass as a production one.

It is the buyer's **only** controller. It is then **blackholed** (its own controller list emptied) so its rules cannot be
changed either; the buyer is upgradable only through it. Before blackholing, anyone can check both canisters' controllers and
the timelock's module hash against its source.

### What it can do, and who may ask

| Action | Who | When | Effect |
| --- | --- | --- | --- |
| `propose_upgrade(wasm, arg)` | proposer | when nothing is pending | stores the module, publishes `sha256(wasm)`, `sha256(arg)` and the arg itself, starts the delay |
| `propose_controllers(list)` | proposer | when nothing is pending | the same, for replacing the buyer's controllers (to move to a new timelock) |
| `cancel(id)` | proposer or guardian | while pending | drops the proposal, logs it |
| `execute(id)` | **anyone** | after the delay | performs it: `install_code` with mode `upgrade` (never `reinstall`, which would wipe the buyer's state), or `update_settings` with only the controller list |
| `pause_target(bool)` | guardian | any time | calls the buyer's `set_paused` |
| `status()`, `history()` | anyone (query) | any time | target, proposer, guardian, delay, the pending proposal's hashes and its earliest execution time, every past event |

One proposal at a time. A proposal whose execution fails (the buyer's `post_upgrade` traps, say — an atomic no-op on the IC)
is recorded as failed and cleared; trying again means proposing again and waiting the full delay again.

**Why `propose_controllers`.** Without it the buyer is bound to this timelock forever: a bug in the timelock, or a longer
delay wanted later, could never be adopted. It gives no power an upgrade doesn't already give — an upgrade can install code
that does anything — and it waits the same public delay.

**Why `execute` is open to anyone.** The delay and the veto are the gates. Letting anyone execute means a proposal nobody
vetoed happens, and nothing depends on the proposer being awake; letting only the proposer execute adds nothing, since the
proposer could propose anything anyway.

### The buyer's side: operators are not controllers

After the handover the operator is not a controller — which, today, would lock them out of every operational call, since
every update method is `only_controller`. So the buyer separates the two:

- **Controllers** (the timelock) change the code. The IC enforces this itself: only a controller can `install_code`.
- **The operator** (a principal in the install argument) runs the buyer: `tick`, `record_quote`, `sign_purchase`, the
  rehearsal methods. `only_operator` admits the operator or a controller, so the buyer works exactly as before until the
  handover.
- **The pause:** the operator may *pause* (stopping is always safe); only a controller may *resume*, so after the handover
  only the guardian, through `pause_target(false)`, can lift a pause. An operator cannot undo a guardian's veto.

The operator is set from the **upgrade argument** on every upgrade (not kept from before, unlike the rest of the install
argument), so changing the operator is itself a timelocked proposal whose argument is published during the delay.

## What this protects against, and what it does not

**Protects:** a stolen or coerced operator key can no longer replace the buyer's code on the spot; the replacement is
visible for at least 72 hours, by hash, with its argument, and the guardian can cancel it; holders who don't trust the
proposer can check the proposed hash against a build of the published source and leave (or raise the alarm) before it lands.
The guardian can stop the buyer at any time and cannot make it do anything else.

**Does not protect against:**

- **What the operator can do within the code.** `sign_purchase` signs only verified PSBTs against recorded quotes; that is the
  code's guarantee, not the timelock's. The timelock governs changes to the rules, not operation under them.
- **A proposer and guardian who collude**, or one person holding both keys. The delay still makes the change public; nothing
  stops it.
- **Nobody watching.** A public delay protects only people who read it. Nothing yet alerts anyone when a proposal appears.
- **The IC itself**: a subnet that misbehaves, or NNS governance acting on the subnet.
- **A bad build that matches its hash.** Reproducibility proves the module came from this source, not that the source is good.

## Reproducible build

The wasm must be byte-for-byte reproducible so that "the hash in the proposal" can be checked against "the source on GitHub":

- `rust-toolchain.toml` pins the compiler and the `wasm32-unknown-unknown` target.
- `cargo build --locked` pins every dependency to `Cargo.lock`.
- `RUSTFLAGS` remaps the absolute source and cargo-home paths, which otherwise end up inside the wasm.
- A `Dockerfile` pins the platform (linux/amd64) and the base image, because the IC's guidance is that Linux and macOS builds
  of the same Rust can differ.
- `scripts/reproducible-build.sh` builds both canisters that way and prints their SHA-256; `verify` compares them with a
  deployed canister's module hash.

**The check.** Two builds from clean checkouts, in different directories, produce the same hash; this is recorded below when
it is run.

## The rehearsal (local replica only)

On the local replica, never mainnet: install a fresh buyer (`autobuyer_timelock`) and a rehearsal timelock (`timelock`);
hand the buyer's control to the timelock; show that the operator can no longer upgrade directly but can still operate and
pause, and cannot resume; propose an upgrade, show its hash in `status()`, refuse an early `execute`, cancel it as guardian;
propose again, wait out the delay, execute it as the anonymous principal, and read the buyer's new module hash — equal to the
proposed one — with its state restored.

## Results (2026-09-23)

### The timelock, on a local replica — **pass**

`autobuyer/timelock-rs/rehearse.sh`, buyer `ll5dv-z7777-77777-aaaca-cai` and timelock `lm4fb-uh777-77777-aaacq-cai` (both new, in
`timelock-rs`'s own dfx project; no other canister touched). Every step asserted, not printed:

1. A delay below the build's floor is refused at install.
2. Handover: the buyer's controllers become exactly `[timelock]`. A direct `install_code` by the former controller, with a
   module whose hash differs from the installed one, is refused and the module is unchanged. (With the *same* module, `dfx`
   skips the install and prints "Upgraded code" anyway — a check that asks the IC nothing. The rehearsal avoids it on purpose.)
3. The operator still operates (reads its EVM address), can pause, **cannot resume**; a stranger cannot even pause; the
   guardian resumes and pauses through `pause_target`.
4. A stranger cannot propose; the proposal's module hash appears in `status()` for the anonymous caller; `execute` before the
   delay is refused; a stranger cannot cancel; the guardian's cancel leaves the buyer untouched.
5. An upgrade that must fail (no install argument, so `post_upgrade` traps) is executed by the anonymous principal after the
   delay, recorded as `Failed`, and leaves the buyer's module hash unchanged: the IC's install is atomic, as the spec says.
6. The real upgrade, executed by the anonymous principal after the delay: the buyer's module hash becomes exactly the
   proposed hash; its log says "upgraded: state restored"; the guardian's pause survived; the timelock is still its only
   controller.
7. `propose_controllers` hands the buyer back after the same delay, executed by the anonymous principal.

And both deployed modules are checked against source with `scripts/reproducible-build.sh verify … local`: **MATCH**.

The proposal in 4–6 is the same code gzipped, so that the module hash visibly changes. That showed something worth knowing:
**the IC's module hash for a gzipped module is the hash of the gzip bytes**, so a gzipped proposal is only verifiable if its
compression is reproducible too (`gzip -9 -n`). Proposals should use the raw reproducible wasm.

### The reproducible build — **pass, on its canonical platform**

Two Docker builds (`scripts/reproducible-build.sh`, linux/amd64, image pinned by digest), each from a fresh clone of commit
`3e75471`, at different paths (`…/reproA` and `…/reproB-at-another/path/etch`), the Docker build cache wiped between them:

| module | build A | build B |
| --- | --- | --- |
| `buyer.wasm` | `df33d082b2d8aa6384515f3650b27edfae7c3c09fedc67370b252e39a170f94e` | same |
| `timelock.wasm` (72 h floor) | `5ae08b69d5681c8925a68242be1242f71b99f882f51806c075a5a8445f51c3f4` | same |
| `timelock-rehearsal.wasm` | `85bbf475c3fed8c3f86a51168213f97ba6ca294b511861452830df6c6fa6ff9d` | same |

Two native macOS (aarch64) builds from clones at different paths also agree with each other (buyer `ef1e3c81…`), which shows
the path remapping works — but **they do not equal the Linux builds**, as the IC's guidance warns. The difference is in the
wasm's name section: cargo's per-crate disambiguator hashes (`Cs…` in the mangled symbols) differ for crates that depend on
proc-macros compiled for the *host*, while `core`'s is identical on both. So the canonical build is the Docker one, and a
verifier must use it.

### How anyone checks a deployed canister

```
git clone <this repo> && cd autobuyer && git checkout <the commit the proposal names>
scripts/reproducible-build.sh                               # Docker, linux/amd64; prints three SHA-256s
dfx canister --network ic info <buyer id>                   # the IC's own, certified: controllers and module hash
scripts/reproducible-build.sh verify <buyer id> buyer ic    # MATCH or NO MATCH
dfx canister --network ic call <timelock id> status         # during a delay: the pending proposal's wasm_hash
```

The controllers line must show only the timelock, and the timelock's own controllers must be empty (blackholed) with its
module hash equal to `timelock.wasm` — not `timelock-rehearsal.wasm`, whose floor is 10 seconds.

### Left open

- **Nothing is deployed beyond the local replica**, and the timelock is not blackholed anywhere. Doing it for real is the
  operator's act: install `timelock.wasm` with a real guardian key held apart from the proposer's, hand over, blackhole.
- **Nothing alerts anyone** when a proposal appears. A public delay protects only people who look; a watcher that posts
  every `Proposed` event is the obvious next piece.
- **Role separation on the replica used one principal** for proposer, guardian and operator (the rehearsal build allows it;
  a production build refuses proposer = guardian). The separation is proved by unit tests, not on the replica, because a
  second identity would have meant generating another key.
- **A module over ~2 MB** cannot be proposed in one message; the IC's chunked install (`upload_chunk` /
  `install_chunked_code` with the timelock as `store_canister`) is the path, and is not built. The buyer is 1.6 MB.
- **The Bitcoin broadcast is still the driver's**, and the buyer's loop is still not wired (`observe` is a stub).

## Superseded in part (2026-09-23): sets, a veto threshold, and roles that change like code

The single `proposer` and single `guardian` above are replaced by `docs/v2/trust-points.md` §3: sets of proposers (≥ 2) and
guardians (≥ 3), a veto that needs k = 2 of them, a pause any guardian can make and k must lift, and a `propose_roles` proposal
that changes the sets after the same 72-hour delay. `cancel` is now `withdraw` (a proposer's own) and `veto` (a guardian's vote);
`pause_target(bool)` is now `pause_target()` and `resume_target()`. A stale proposal — made before another target action or roles
change ran — fails. The delay floor, the test-build separation, `execute` open to anyone, upgrade-never-reinstall, the blackhole and
the reproducible build are unchanged. Both rehearsals pass on the new interface (`rehearse.sh`, `rehearse-roles.sh`).
