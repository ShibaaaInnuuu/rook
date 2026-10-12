//! The timelock as a canister: state in a cell, [`crate::rules`] consulted on every call, and the management canister called
//! only once the rules have said yes. It calls the management methods `stop_canister`, `install_code` (mode `upgrade`) and
//! `start_canister` for an upgrade (audit D, L-2: the buyer is stopped first, so no tick's outcalls are cut off by the new
//! code), and `update_settings` (controllers only); for a module too large for one message (W14a, [`crate::rules::install_plan`]),
//! `clear_chunk_store` and `upload_chunk` on the TARGET's chunk store before the stop and `install_chunked_code` (mode `upgrade`,
//! the full module's hash) in place of `install_code`; and one method on the target, `set_paused`. There is no other outbound call.

use crate::rules::{Action, Config, Event, InstallPlan, ProposalView, Ready, Roles, State};
use candid::{CandidType, Principal};
use ic_cdk::management_canister::{
    clear_chunk_store, install_chunked_code, install_code, start_canister, stop_canister, update_settings, upload_chunk, CanisterInstallMode,
    CanisterSettings, ChunkHash, ClearChunkStoreArgs, InstallChunkedCodeArgs, InstallCodeArgs, StartCanisterArgs, StopCanisterArgs,
    UpdateSettingsArgs, UpgradeFlags, UploadChunkArgs,
};
use serde::Deserialize;
use std::cell::RefCell;

/// Below this many cycles the watcher alerts (`docs/v2/trust-points.md` §2). The timelock spends almost nothing; it must only
/// never freeze, because a frozen timelock cannot execute or veto.
pub const LOW_CYCLES: u128 = 500_000_000_000;

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn with<T>(f: impl FnOnce(&mut State) -> T) -> T {
    STATE.with(|s| f(s.borrow_mut().as_mut().expect("not installed")))
}

fn caller() -> Principal {
    ic_cdk::api::msg_caller()
}

fn now() -> u64 {
    ic_cdk::api::time()
}

#[ic_cdk::init]
fn init(cfg: Config) {
    let s = State::new(cfg).unwrap_or_else(|e| ic_cdk::trap(&e));
    STATE.with(|x| *x.borrow_mut() = Some(s));
}

/// In production the timelock is blackholed after install and is never upgraded. On a replica it may be; its state, pending
/// proposals included, survives.
#[ic_cdk::pre_upgrade]
fn pre_upgrade() {
    let s = STATE.with(|x| x.borrow().clone());
    ic_cdk::storage::stable_save((s,)).expect("could not save the timelock's state; refusing the upgrade");
}

#[ic_cdk::post_upgrade]
fn post_upgrade(cfg: Config) {
    match ic_cdk::storage::stable_restore::<(Option<State>,)>() {
        Ok((Some(s),)) => {
            // The install argument is validated but does not replace the saved state: an upgrade of the timelock is not a way
            // to change its target, roles or delay. Roles change only through a `Roles` proposal.
            cfg.validate().unwrap_or_else(|e| ic_cdk::trap(&e));
            STATE.with(|x| *x.borrow_mut() = Some(s));
        }
        // saved as never installed: installing now is right
        Ok((None,)) => init(cfg),
        // audit D, M-2: a state that does not decode is never a fresh install (that would forget pending proposals and the
        // roles). Trapping refuses the upgrade and keeps everything.
        Err(e) => ic_cdk::trap(&format!("refused: the saved state does not decode into this build's types ({e}); nothing changed")),
    }
}

/// Audit D, L-3: ingress the rules would only refuse is not accepted (`rules::State::admits`), so strangers cannot spend the
/// timelock's cycles on refused calls. Queries never pass through here.
#[ic_cdk::inspect_message]
fn inspect_message() {
    let (c, m) = (caller(), ic_cdk::api::msg_method_name());
    let id = if m == "execute" { candid::decode_one::<u64>(&ic_cdk::api::msg_arg_data()).ok() } else { None };
    let ok = ic_cdk::api::is_controller(&c) || STATE.with(|s| s.borrow().as_ref().is_some_and(|s| s.admits(c, &m, id, now())));
    if ok {
        ic_cdk::api::accept_message();
    }
}

#[derive(CandidType, Deserialize)]
pub struct Status {
    pub target: Principal,
    pub roles: Roles,
    pub delay_secs: u64,
    pub min_delay_secs: u64,
    pub rehearsal_build: bool,
    pub pending: Vec<ProposalView>,
    /// Target proposals being executed right now (audit R2, N-6), each with what cancelled it, if anything did.
    pub running: Vec<(ProposalView, Option<String>)>,
    pub target_epoch: u64,
    pub roles_epoch: u64,
    pub resume_votes: Vec<Principal>,
    pub now_ns: u64,
}

/// Everything a holder needs to judge what is about to happen, readable by anyone.
#[ic_cdk::query]
fn status() -> Status {
    with(|s| Status {
        target: s.target,
        roles: s.roles.clone(),
        delay_secs: s.delay_secs,
        min_delay_secs: crate::rules::MIN_DELAY_SECS,
        rehearsal_build: cfg!(feature = "rehearsal"),
        pending: s.pending.iter().map(|p| p.view()).collect(),
        running: s.running.iter().flatten().map(|r| (r.proposal.view(), r.cancelled.clone())).collect(),
        target_epoch: s.target_epoch,
        roles_epoch: s.roles_epoch,
        resume_votes: s.resume_votes.clone(),
        now_ns: now(),
    })
}

#[ic_cdk::query]
fn history() -> Vec<Event> {
    with(|s| s.history.clone())
}

#[derive(CandidType, Deserialize)]
pub struct Cycles {
    pub balance: candid::Nat,
    pub low_threshold: candid::Nat,
}

/// The timelock's own balance, for anyone: after it is blackholed nobody can call `canister_status` on it.
#[ic_cdk::query]
fn cycles() -> Cycles {
    Cycles { balance: ic_cdk::api::canister_cycle_balance().into(), low_threshold: LOW_CYCLES.into() }
}

/// Any proposer (one pending each). The module is stored here and its hash published; nothing happens to the target until
/// `execute`.
#[ic_cdk::update]
fn propose_upgrade(wasm: serde_bytes::ByteBuf, arg: serde_bytes::ByteBuf) -> Result<ProposalView, String> {
    let (c, t) = (caller(), now());
    with(|s| s.propose(c, Action::Upgrade { wasm: wasm.into_vec(), arg: arg.into_vec() }, t))
}

/// W14a: any proposer stages a module too large for one message, one piece at a time (each at most `rules::MAX_STAGE_CHUNK`). Answers the
/// bytes staged so far and their SHA-256 (hex). Nothing is proposed, published or waited on until `propose_staged_upgrade`.
#[ic_cdk::update]
fn stage_chunk(chunk: serde_bytes::ByteBuf) -> Result<(u64, String), String> {
    let (c, t) = (caller(), now());
    with(|s| s.stage(c, &chunk, t))
}

/// W14a: the caller's staging dropped.
#[ic_cdk::update]
fn unstage() -> Result<(), String> {
    let c = caller();
    with(|s| s.unstage(c))
}

/// W14a: what `who` has staged (size, SHA-256 hex), for anyone.
#[ic_cdk::query]
fn staged(who: Principal) -> Option<(u64, String)> {
    with(|s| s.staged_of(who))
}

/// W14a: any proposer proposes the module it staged, whole: `sha256` must be the full module's hash (hex), what the proposal publishes
/// and the target's module hash after execution. The same delay and veto as `propose_upgrade`.
#[ic_cdk::update]
fn propose_staged_upgrade(sha256: String, arg: serde_bytes::ByteBuf) -> Result<ProposalView, String> {
    let (c, t) = (caller(), now());
    with(|s| s.propose_staged(c, &sha256, arg.into_vec(), t))
}

/// W14a: a large module's pieces into the target's chunk store (cleared first), each hash checked against the plan's.
async fn upload_pieces(target: Principal, wasm: &[u8], pieces: &[(usize, usize)], hashes: &[[u8; 32]]) -> Result<(), String> {
    clear_chunk_store(&ClearChunkStoreArgs { canister_id: target }).await.map_err(|e| format!("clear_chunk_store: {e:?}"))?;
    for (i, ((a, b), want)) in pieces.iter().zip(hashes).enumerate() {
        let got = upload_chunk(&UploadChunkArgs { canister_id: target, chunk: wasm[*a..*b].to_vec() }).await
            .map_err(|e| format!("upload_chunk {i}: {e:?}"))?;
        if got.hash.as_slice() != want.as_slice() {
            return Err(format!("upload_chunk {i}: the IC stored a chunk hashed {}, not {}", hex::encode(&got.hash), hex::encode(want)));
        }
    }
    Ok(())
}

/// Any proposer: replace the target's controllers (to move it to a new timelock), after the same delay.
#[ic_cdk::update]
fn propose_controllers(list: Vec<Principal>) -> Result<ProposalView, String> {
    let (c, t) = (caller(), now());
    with(|s| s.propose(c, Action::Controllers { list }, t))
}

/// Any proposer: replace the proposers, the guardians and the veto threshold, after the same delay. Vetoable like any other.
#[ic_cdk::update]
fn propose_roles(roles: Roles) -> Result<ProposalView, String> {
    let (c, t) = (caller(), now());
    with(|s| s.propose(c, Action::Roles { roles }, t))
}

/// The proposer of `id`, and nobody else.
#[ic_cdk::update]
fn withdraw(id: u64) -> Result<(), String> {
    let (c, t) = (caller(), now());
    with(|s| s.withdraw(c, id, t))
}

/// Any guardian's vote against `id`; at the veto threshold it is gone. Answers whether it was vetoed.
#[ic_cdk::update]
fn veto(id: u64) -> Result<bool, String> {
    let (c, t) = (caller(), now());
    with(|s| s.veto(c, id, t))
}

/// Anyone, after the delay. The proposal leaves the state before the management canister is awaited, so it runs once. A roles
/// change is applied inside the state and awaits nothing; a stale proposal is cleared and recorded as failed.
#[ic_cdk::update]
async fn execute(id: u64) -> Result<String, String> {
    let (by, t) = (caller(), now());
    let ready = match with(|s| s.take_ready(id, t)) {
        Ok(r) => r,
        Err(why) if why.starts_with("stale") => {
            with(|s| s.record(Event::Failed { id, at_ns: t, by, why: why.clone() }));
            return Err(format!("proposal {id} failed and was cleared: {why}"));
        }
        Err(why) => return Err(why),
    };
    let p = match ready {
        Ready::RolesApplied(_) => {
            with(|s| s.record(Event::Executed { id, at_ns: t, by }));
            return Ok(format!("proposal {id} executed: the roles are changed"));
        }
        Ready::Target(p) => p,
    };
    let target = with(|s| s.target);
    let done = match p.action.clone() {
        Action::Upgrade { wasm, arg } => {
            // W14a: a module too large for one message goes through the target's chunk store, uploaded BEFORE the stop (the buyer keeps
            // running meanwhile); an upload that fails stops and installs nothing, and the proposal is pending again
            let plan = crate::rules::install_plan(&wasm, &arg);
            if let InstallPlan::Chunked { pieces, hashes, .. } = &plan {
                if let Err(why) = upload_pieces(target, &wasm, pieces, hashes).await {
                    let at = now();
                    let why = format!("the module's chunks were not uploaded ({why}); nothing was stopped or installed");
                    let back = with(|s| {
                        let back = s.untake(id, at).unwrap_or(false);
                        s.record(Event::NotUploaded { id, at_ns: at, why: why.clone() });
                        back
                    });
                    return Err(if back {
                        format!("proposal {id} did not run and is pending again (runnable in {} s): {why}", crate::rules::STOP_RETRY_NS / 1_000_000_000)
                    } else {
                        format!("proposal {id} did not run and was dropped (cancelled meanwhile): {why}")
                    });
                }
            }
            // L-2 (audit D): stop the buyer first. Stopping waits for its outstanding calls (a tick's outcalls are bounded), so
            // no continuation is cut off by the new code. A stop that does not finish installs nothing: the buyer is started
            // again and the proposal goes back to pending, if it is still wanted (audit R2, N-6).
            if let Err(e) = stop_canister(&StopCanisterArgs { canister_id: target }).await {
                let why = format!("the buyer did not stop ({e:?}); nothing was installed");
                let started = start_canister(&StartCanisterArgs { canister_id: target }).await;
                let at = now();
                let back = with(|s| {
                    let back = s.untake(id, at).unwrap_or(false);
                    s.record(Event::NotStopped { id, at_ns: at, why: why.clone() });
                    if let Err(e) = &started {
                        s.record(Event::NotStarted { id, at_ns: at, why: format!("after a failed stop, the buyer did not start: {e:?}") });
                    }
                    back
                });
                return Err(if back {
                    format!("proposal {id} did not run and is pending again (runnable in {} s): {why}", crate::rules::STOP_RETRY_NS / 1_000_000_000)
                } else {
                    format!("proposal {id} did not run and was dropped (cancelled while the buyer was stopping): {why}")
                });
            }
            // audit R2, N-6: a veto, a withdrawal or a roles change may have landed while the stop was awaited
            // audit R3, R3-C5: still wanted, and from here committed (no await between): a veto that lands during the install is
            // refused rather than recorded as if it had stopped it
            if let Err(cancelled) = with(|s| s.still_wanted(id).and_then(|()| s.commit(id))) {
                let started = start_canister(&StartCanisterArgs { canister_id: target }).await;
                let at = now();
                let why = format!("{cancelled} while the buyer was stopping: nothing was installed");
                with(|s| {
                    let _ = s.untake(id, at);
                    s.record(Event::Failed { id, at_ns: at, by, why: why.clone() });
                    if let Err(e) = &started {
                        s.record(Event::NotStarted { id, at_ns: at, why: format!("after a cancelled upgrade, the buyer did not start: {e:?}") });
                    }
                });
                return Err(format!("proposal {id} was not installed: {why}"));
            }
            // upgrade, never reinstall: the buyer's purchases, pause and log must survive
            let mode = CanisterInstallMode::Upgrade(Some(UpgradeFlags { skip_pre_upgrade: Some(false), wasm_memory_persistence: None }));
            let installed = match plan {
                InstallPlan::Single => install_code(&InstallCodeArgs { mode, canister_id: target, wasm_module: wasm, arg }).await
                    .map_err(|e| format!("{e:?}")),
                // the IC checks every chunk's hash and the assembled module's against `wasm_module_hash` (the FULL module's: the
                // published hash), so what installs is exactly what was proposed
                InstallPlan::Chunked { hashes, module, .. } => install_chunked_code(&InstallChunkedCodeArgs { mode, target_canister: target,
                    store_canister: None, chunk_hashes_list: hashes.iter().map(|h| ChunkHash { hash: h.to_vec() }).collect(),
                    wasm_module_hash: module.to_vec(), arg }).await
                    .map_err(|e| format!("{e:?}")),
            };
            // the chunk store is the target's memory: emptied whatever the install did (best effort; the next upload clears it anyway)
            let _ = clear_chunk_store(&ClearChunkStoreArgs { canister_id: target }).await;
            // started again whatever the install did: a refused upgrade left the old code, which must keep running
            let started = start_canister(&StartCanisterArgs { canister_id: target }).await.map_err(|e| format!("{e:?}"));
            if let Err(e) = &started {
                // audit R2, N-6: recorded whatever the install did; a guardian's `start_target` is the way out
                let at = now();
                let what = if installed.is_ok() { "installed" } else { "the install failed" };
                with(|s| s.record(Event::NotStarted { id, at_ns: at, why: format!("{what}, and the buyer did not start again: {e}") }));
            }
            installed
        }
        Action::Controllers { list } => update_settings(&UpdateSettingsArgs {
            canister_id: target,
            settings: CanisterSettings { controllers: Some(list), ..Default::default() },
        })
        .await
        .map_err(|e| format!("{e:?}")),
        Action::Roles { .. } => unreachable!("a roles change is applied by take_ready"),
    };
    let at = now();
    with(|s| s.finish(id));
    match done {
        Ok(()) => {
            with(|s| s.record(Event::Executed { id, at_ns: at, by }));
            Ok(format!("proposal {id} executed"))
        }
        // The IC's install is atomic: a rejected upgrade changed nothing. The epoch still moved (the proposal was taken before the
        // await), so any other pending target action made before it is stale and must be proposed again: the conservative side.
        Err(why) => {
            with(|s| s.record(Event::Failed { id, at_ns: at, by, why: why.clone() }));
            Err(format!("proposal {id} failed and was cleared: {why}"))
        }
    }
}

async fn set_target_paused(paused: bool) -> Result<(), String> {
    let target = with(|s| s.target);
    let r = ic_cdk::call::Call::unbounded_wait(target, "set_paused").with_arg(paused).await;
    let at = now();
    let why = r.err().map(|e| format!("{e:?}"));
    with(|s| s.record(Event::Paused { at_ns: at, paused, ok: why.is_none(), why: why.clone() }));
    match why {
        None => Ok(()),
        Some(w) => Err(format!("the buyer refused or failed: {w}")),
    }
}

/// Any guardian: stop the buyer. Stopping is always safe, so one guardian is enough.
#[ic_cdk::update]
async fn pause_target() -> Result<(), String> {
    let c = caller();
    with(|s| s.may_pause(c))?;
    set_target_paused(true).await
}

/// Any guardian: start the buyer (audit R2, N-6), for when an upgrade's install and its restart both failed and it stays
/// Stopped. Starting is safe: a paused buyer stays paused (its own flag), and the code is whatever is installed. Refused while a
/// proposal is being executed (it would break that execute's stop).
#[ic_cdk::update]
async fn start_target() -> Result<(), String> {
    let c = caller();
    let t = now();
    with(|s| s.may_start(c, t))?;
    let target = with(|s| s.target);
    let r = start_canister(&StartCanisterArgs { canister_id: target }).await;
    let at = now();
    let why = r.err().map(|e| format!("{e:?}"));
    with(|s| s.record(Event::Started { at_ns: at, by: c, ok: why.is_none(), why: why.clone() }));
    match why {
        None => Ok(()),
        Some(w) => Err(format!("the buyer did not start: {w}")),
    }
}

/// A guardian's vote to resume the buyer; at the veto threshold it is resumed. After the handover this is the only way to
/// resume it. Answers whether it was resumed.
#[ic_cdk::update]
async fn resume_target() -> Result<bool, String> {
    let (c, t) = (caller(), now());
    if !with(|s| s.resume_vote(c, t))? {
        return Ok(false);
    }
    set_target_paused(false).await.map(|()| true)
}

#[cfg(test)]
mod tests {
    /// Every update method is accounted for: its guard is named here, so an update added later without deciding who may call
    /// it fails this test rather than shipping open.
    #[test]
    fn every_update_method_is_one_the_design_names() {
        let src = include_str!("canister.rs");
        let needle = format!("#[ic_cdk::{}]", "update");
        let names: Vec<String> = src.split(needle.as_str()).skip(1)
            .map(|part| part.split("fn ").nth(1).and_then(|s| s.split('(').next()).unwrap_or("?").trim().to_string())
            .collect();
        assert_eq!(names, ["propose_upgrade", "stage_chunk", "unstage", "propose_staged_upgrade", "propose_controllers", "propose_roles",
            "withdraw", "veto", "execute", "pause_target", "start_target", "resume_target"],
            "propose_*, stage_chunk, unstage: any proposer (rules::propose, rules::stage); withdraw: its own proposer; veto: a guardian's vote; execute: anyone after the \
             delay; pause_target: any guardian; start_target: any guardian, never mid-execute; resume_target: a guardian's vote");
    }
}
