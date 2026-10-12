//! Every decision the timelock makes, with time passed in. Nothing here calls anything.
//!
//! `docs/v2/timelock-controller.md` is the design; `docs/v2/trust-points.md` §3 replaced its single proposer and single guardian
//! with sets, a veto threshold and a third kind of proposal (`Roles`), so that no single key, lost or stolen, can freeze or redirect
//! the buyer for longer than one delay.

use candid::{CandidType, Principal};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The shortest delay this build accepts. 72 hours in a production build: three days in which anyone can read a proposal's
/// hash and the guardians can veto it. The `rehearsal` feature lowers it for replica tests, and changes the module hash.
#[cfg(not(feature = "rehearsal"))]
pub const MIN_DELAY_SECS: u64 = 72 * 3600;
#[cfg(feature = "rehearsal")]
pub const MIN_DELAY_SECS: u64 = 10;

/// The IC allows at most 10 controllers on a canister.
pub const MAX_CONTROLLERS: usize = 10;

/// A module proposed in ONE message (`propose_upgrade`) must fit in one ingress message (2 MiB, less the call's own envelope).
pub const MAX_MODULE_BYTES: usize = 2_000_000;

/// W14a (docs/v9/canister-only-solana.md §13.1, D-d): a module too large for one message (the buyer is ≈ 1.8 MB gzipped, near the 2 MiB
/// limit) is STAGED by its proposer in pieces (`stage_chunk`, each at most [`MAX_STAGE_CHUNK`]) and proposed whole
/// (`propose_staged_upgrade`, naming the SHA-256 of the full module it means). Nothing changes for the delay or the veto: the proposal is
/// the full module, its published hash is over the full module (the target's `module_hash` after execution, what a reproducible build
/// is compared with), and it waits the same delay from the moment it is proposed. At most this many bytes.
pub const MAX_CHUNKED_MODULE_BYTES: usize = 16_000_000;
/// One staged piece, at most (one ingress message, less its envelope).
pub const MAX_STAGE_CHUNK: usize = 1_900_000;
/// The IC's chunk store takes chunks of at most 1 MiB (`upload_chunk`).
pub const IC_CHUNK_BYTES: usize = 1_048_576;
/// A module plus its argument up to this size is installed with one `install_code` call (an inter-canister message of at most 2 MiB);
/// a larger one through the target's chunk store (`upload_chunk` × n, `install_chunked_code`).
pub const SINGLE_INSTALL_BYTES: usize = 1_900_000;

/// At most this many proposers, and this many guardians.
pub const MAX_ROLE: usize = 5;

const NS: u64 = 1_000_000_000;

/// A proposal whose buyer did not stop goes back to pending runnable only after this long (audit R2, N-6), so a stop that keeps
/// failing is not retried in a loop by anyone.
pub const STOP_RETRY_NS: u64 = 10 * 60 * NS;

/// A proposal still listed as running this long after `execute` took it is an orphan (audit R3, R3-C5): an execute that trapped
/// after an await left it behind (a real one takes minutes). A guardian's `start_target` clears it, recorded as `Failed`.
pub const RUNNING_ORPHAN_NS: u64 = 24 * 3600 * NS;

/// A round of resume votes lasts this long from its first vote (audit D, I-4): a guardian's vote from an earlier incident does
/// not count toward resuming after a later pause the timelock never saw (a tripwire, or the operator's).
pub const RESUME_VOTE_TTL_NS: u64 = 12 * 3600 * NS;

/// Who may do what. Changed only by a `Roles` proposal, which waits the same delay and can be vetoed like any other.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Roles {
    /// Any one of them may propose; each has at most one pending proposal and may withdraw only its own.
    pub proposers: Vec<Principal>,
    /// A proposal is vetoed when `veto_threshold` of them have voted against it. Any one may pause the buyer; resuming takes
    /// `veto_threshold` of them.
    pub guardians: Vec<Principal>,
    pub veto_threshold: u8,
}

impl Roles {
    /// The same rules at install and for every `Roles` proposal. A production build demands that no single key, lost or
    /// stolen, can freeze the buyer: two proposers at least, three guardians at least, and a veto that needs two of them but
    /// survives losing one.
    pub fn validate(&self) -> Result<(), String> {
        let anon = Principal::anonymous();
        let (p, g, k) = (self.proposers.len(), self.guardians.len(), self.veto_threshold as usize);
        if p == 0 || g == 0 {
            return Err("refused: at least one proposer and one guardian".into());
        }
        if p > MAX_ROLE || g > MAX_ROLE {
            return Err(format!("refused: at most {MAX_ROLE} proposers and {MAX_ROLE} guardians"));
        }
        if self.proposers.contains(&anon) || self.guardians.contains(&anon) {
            return Err("refused: the anonymous principal cannot be a proposer or a guardian".into());
        }
        let twice = |v: &Vec<Principal>| v.iter().enumerate().any(|(i, x)| v[..i].contains(x));
        if twice(&self.proposers) || twice(&self.guardians) {
            return Err("refused: a principal listed twice in one role".into());
        }
        if k == 0 || k > g {
            return Err(format!("refused: a veto threshold of {k} with {g} guardians"));
        }
        if !cfg!(feature = "rehearsal") {
            if p < 2 {
                return Err("refused: a single proposer, lost, would mean no upgrade ever again; name two at least".into());
            }
            if g < 3 {
                return Err("refused: fewer than three guardians; a veto that needs two must survive losing one".into());
            }
            if k < 2 || k > g - 1 {
                return Err(format!(
                    "refused: a veto threshold of {k} with {g} guardians; it must be at least 2 (no single guardian can freeze the \
                     buyer) and at most {} (losing one guardian must not lose the veto)", g - 1));
            }
            if self.proposers.iter().any(|x| self.guardians.contains(x)) {
                return Err("refused: a principal is both a proposer and a guardian, so part of the veto would be the proposer's own".into());
            }
        }
        Ok(())
    }
}

/// What the timelock is installed with. `target` and `delay_secs` never change; `roles` change only through a `Roles` proposal.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// The buyer: the only canister this acts on.
    pub target: Principal,
    pub roles: Roles,
    /// Seconds between a proposal and the earliest moment anyone may execute it.
    pub delay_secs: u64,
}

impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if self.target == Principal::anonymous() {
            return Err("refused: the anonymous principal cannot be the target".into());
        }
        if self.delay_secs < MIN_DELAY_SECS {
            return Err(format!("refused: a delay of {} s is below this build's floor of {} s", self.delay_secs, MIN_DELAY_SECS));
        }
        self.roles.validate()
    }
}

/// What a proposal does when executed.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// `install_code` on the target, mode `upgrade` (never `reinstall`, which would erase its state).
    Upgrade {
        #[serde(with = "serde_bytes")]
        wasm: Vec<u8>,
        #[serde(with = "serde_bytes")]
        arg: Vec<u8>,
    },
    /// `update_settings` on the target, changing only its controller list: how the buyer moves to a new timelock.
    Controllers { list: Vec<Principal> },
    /// Replace the proposers, the guardians and the veto threshold. The target is untouched.
    Roles { roles: Roles },
}

impl Action {
    fn touches_target(&self) -> bool {
        !matches!(self, Action::Roles { .. })
    }
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    pub id: u64,
    pub by: Principal,
    pub action: Action,
    pub proposed_ns: u64,
    pub executable_ns: u64,
    /// How many target actions (upgrades, controller changes) had executed when this was proposed: a target action runs only if
    /// none has run since, so two pending upgrades cannot land one after the other and undo each other.
    pub target_epoch: u64,
    /// How many roles changes had executed when this was proposed: a roles change runs only on an up-to-date picture.
    pub roles_epoch: u64,
    /// Guardians who have voted to veto it, so far.
    pub vetoes: Vec<Principal>,
}

/// A proposal as anyone may read it: hashes, the argument itself, and when it can happen. The module bytes are not
/// returned (1.6 MB); their hash is what a reader checks against a reproducible build.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ProposalView {
    pub id: u64,
    pub by: Principal,
    pub kind: String,
    /// SHA-256 of the module, hex: after execution this is the target's `module_hash`.
    pub wasm_hash: Option<String>,
    pub wasm_bytes: Option<u64>,
    /// The upgrade argument, hex, and its SHA-256. It sets the buyer's operator and endpoints, so it is published whole.
    pub arg_hex: Option<String>,
    pub arg_hash: Option<String>,
    pub controllers: Option<Vec<Principal>>,
    pub roles: Option<Roles>,
    pub proposed_ns: u64,
    pub executable_ns: u64,
    pub vetoes: Vec<Principal>,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Proposed { id: u64, at_ns: u64, view: ProposalView },
    /// Withdrawn by its own proposer.
    Cancelled { id: u64, at_ns: u64, by: Principal },
    /// One guardian's vote against a proposal; `votes` of `needed` so far.
    VetoVote { id: u64, at_ns: u64, by: Principal, votes: u8, needed: u8 },
    /// Enough guardians voted: the proposal is gone.
    Vetoed { id: u64, at_ns: u64, by: Vec<Principal> },
    Executed { id: u64, at_ns: u64, by: Principal },
    Failed { id: u64, at_ns: u64, by: Principal, why: String },
    Paused { at_ns: u64, paused: bool, ok: bool, why: Option<String> },
    /// One guardian's vote to resume the buyer; resuming takes `needed`.
    ResumeVote { at_ns: u64, by: Principal, votes: u8, needed: u8 },
    RolesChanged { id: u64, at_ns: u64, roles: Roles },
    /// The buyer did not stop before an upgrade (audit D, L-2): nothing was installed, the buyer was started again, and the
    /// proposal is pending again, runnable as before.
    NotStopped { id: u64, at_ns: u64, why: String },
    /// The buyer did not start again after an upgrade's stop (audit D, L-2; audit R2, N-6: recorded whatever the install did):
    /// it must be started (a watcher alert), by a guardian's `start_target`.
    NotStarted { id: u64, at_ns: u64, why: String },
    /// A guardian started the buyer (audit R2, N-6), and whether the management canister did.
    Started { at_ns: u64, by: Principal, ok: bool, why: Option<String> },
    /// W14a: a large module's chunks were not uploaded to the target's chunk store before its stop: nothing was stopped or installed,
    /// and the proposal is pending again, runnable as after a stop that failed.
    NotUploaded { id: u64, at_ns: u64, why: String },
}

/// W14a: a module being staged by its proposer, piece by piece, before it is proposed whole. Not a proposal: nothing waits on it, and
/// nothing is published but its size and running hash.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Staged {
    pub by: Principal,
    #[serde(with = "serde_bytes")]
    pub bytes: Vec<u8>,
    pub since_ns: u64,
}

/// W14a: how a module is installed on the target (pure, from the module and its argument).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallPlan {
    /// One `install_code` call.
    Single,
    /// The target's chunk store: `clear_chunk_store`, `upload_chunk` of each `[start, end)` piece (each hash checked against `hashes`),
    /// then `install_chunked_code` naming the hashes in order and the full module's SHA-256 (`module`).
    Chunked { pieces: Vec<(usize, usize)>, hashes: Vec<[u8; 32]>, module: [u8; 32] },
}

/// The install plan of `wasm` with `arg` (pure).
pub fn install_plan(wasm: &[u8], arg: &[u8]) -> InstallPlan {
    if wasm.len() + arg.len() <= SINGLE_INSTALL_BYTES {
        return InstallPlan::Single;
    }
    let pieces: Vec<(usize, usize)> = (0..wasm.len()).step_by(IC_CHUNK_BYTES).map(|a| (a, (a + IC_CHUNK_BYTES).min(wasm.len()))).collect();
    let hashes = pieces.iter().map(|(a, b)| Sha256::digest(&wasm[*a..*b]).into()).collect();
    InstallPlan::Chunked { pieces, hashes, module: Sha256::digest(wasm).into() }
}

/// A target proposal taken by `take_ready` whose `execute` has not finished (audit R2, N-6): still visible, so a veto, a
/// withdrawal or a roles change that lands while the buyer is being stopped is not lost.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Running {
    pub proposal: Proposal,
    /// Why it must not install after all, when something cancelled it meanwhile.
    pub cancelled: Option<String>,
    /// When `execute` took it (audit R3, R3-C5). `None` in a state saved before it existed: counted as an orphan.
    #[serde(default)]
    pub since_ns: Option<u64>,
    /// `Some(true)` once `execute` passed its last check and is installing (after the stop for an upgrade; at once for a
    /// controllers change, whose single await has no later check). A veto or withdrawal can no longer stop it, so it is refused
    /// rather than recorded as if it had (audit R3, R3-C5).
    #[serde(default)]
    pub committed: Option<bool>,
}

impl Running {
    fn committed(&self) -> bool {
        self.committed == Some(true)
    }
}

pub fn sha256_hex(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

impl Proposal {
    pub fn view(&self) -> ProposalView {
        let (kind, wasm_hash, wasm_bytes, arg_hex, arg_hash, controllers, roles) = match &self.action {
            Action::Upgrade { wasm, arg } => ("upgrade", Some(sha256_hex(wasm)), Some(wasm.len() as u64), Some(hex::encode(arg)),
                Some(sha256_hex(arg)), None, None),
            Action::Controllers { list } => ("controllers", None, None, None, None, Some(list.clone()), None),
            Action::Roles { roles } => ("roles", None, None, None, None, None, Some(roles.clone())),
        };
        ProposalView { id: self.id, by: self.by, kind: kind.into(), wasm_hash, wasm_bytes, arg_hex, arg_hash, controllers, roles,
            proposed_ns: self.proposed_ns, executable_ns: self.executable_ns, vetoes: self.vetoes.clone() }
    }
}

/// What `take_ready` hands the caller: something to do to the target, or a roles change already applied.
#[derive(Debug, PartialEq, Eq)]
pub enum Ready {
    Target(Proposal),
    RolesApplied(Proposal),
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub target: Principal,
    pub delay_secs: u64,
    pub roles: Roles,
    pub pending: Vec<Proposal>,
    pub next_id: u64,
    pub target_epoch: u64,
    pub roles_epoch: u64,
    /// Guardians who have voted to resume a paused buyer since it was last paused.
    pub resume_votes: Vec<Principal>,
    pub history: Vec<Event>,
    /// When the current round of resume votes began (audit D, I-4). `None` in a state saved before it existed.
    #[serde(default)]
    pub resume_round_ns: Option<u64>,
    /// Target proposals being executed (audit R2, N-6). `None` in a state saved before it existed.
    #[serde(default)]
    pub running: Option<Vec<Running>>,
    /// W14a: modules being staged, one per proposer. `None` in a state saved before it existed.
    #[serde(default)]
    pub staged: Option<Vec<Staged>>,
}

impl State {
    pub fn new(cfg: Config) -> Result<Self, String> {
        cfg.validate()?;
        Ok(State { target: cfg.target, delay_secs: cfg.delay_secs, roles: cfg.roles, pending: Vec::new(), next_id: 1,
            target_epoch: 0, roles_epoch: 0, resume_votes: Vec::new(), history: Vec::new(), resume_round_ns: None, running: None, staged: None })
    }

    fn is_guardian(&self, p: &Principal) -> bool {
        self.roles.guardians.contains(p)
    }

    /// Any proposer, one pending proposal each, only something the IC (or these rules) would accept.
    pub fn propose(&mut self, caller: Principal, action: Action, now_ns: u64) -> Result<ProposalView, String> {
        if !self.roles.proposers.contains(&caller) {
            return Err(format!("refused: {caller} is not a proposer"));
        }
        if let Some(p) = self.pending.iter().find(|p| p.by == caller) {
            return Err(format!("refused: {caller} already has proposal {} pending; withdraw it or let it run first", p.id));
        }
        match &action {
            Action::Upgrade { wasm, .. } => {
                let raw = wasm.starts_with(b"\0asm");
                let gz = wasm.starts_with(&[0x1f, 0x8b, 0x08]);
                if !raw && !gz {
                    return Err("refused: not a WebAssembly module (no \\0asm header) and not gzip".into());
                }
                if wasm.len() > MAX_CHUNKED_MODULE_BYTES {
                    return Err(format!("refused: {} bytes; a module is at most {MAX_CHUNKED_MODULE_BYTES}", wasm.len()));
                }
            }
            Action::Controllers { list } => {
                if list.len() > MAX_CONTROLLERS {
                    return Err(format!("refused: {} controllers; the IC allows {MAX_CONTROLLERS}", list.len()));
                }
                if list.contains(&Principal::anonymous()) {
                    return Err("refused: the anonymous principal as a controller is a controller anyone can use".into());
                }
                if list.iter().enumerate().any(|(i, p)| list[..i].contains(p)) {
                    return Err("refused: a controller listed twice".into());
                }
            }
            Action::Roles { roles } => roles.validate()?,
        }
        let id = self.next_id;
        self.next_id += 1;
        let p = Proposal { id, by: caller, action, proposed_ns: now_ns, executable_ns: now_ns + self.delay_secs * NS,
            target_epoch: self.target_epoch, roles_epoch: self.roles_epoch, vetoes: Vec::new() };
        let view = p.view();
        self.record(Event::Proposed { id, at_ns: now_ns, view: view.clone() });
        self.pending.push(p);
        Ok(view)
    }

    fn index_of(&self, id: u64) -> Result<usize, String> {
        self.pending.iter().position(|p| p.id == id).ok_or_else(|| format!("refused: proposal {id} is not pending"))
    }

    /// A proposer may withdraw its OWN proposal, and nothing else: a stolen proposer cannot clear the others'.
    pub fn withdraw(&mut self, caller: Principal, id: u64, now_ns: u64) -> Result<(), String> {
        // audit R2, N-6: one being executed is cancelled, if its buyer has not been installed yet
        if let Some(r) = self.running_mut(id) {
            if r.proposal.by != caller {
                return Err(format!("refused: proposal {id} is not {caller}'s to withdraw"));
            }
            if r.committed() {
                return Err(format!("refused: proposal {id} is already being installed; a withdrawal can no longer stop it"));
            }
            r.cancelled.get_or_insert_with(|| "withdrawn by its proposer".into());
            self.record(Event::Cancelled { id, at_ns: now_ns, by: caller });
            return Ok(());
        }
        let i = self.index_of(id)?;
        if self.pending[i].by != caller {
            return Err(format!("refused: proposal {id} is not {caller}'s to withdraw"));
        }
        self.pending.remove(i);
        self.record(Event::Cancelled { id, at_ns: now_ns, by: caller });
        Ok(())
    }

    /// One guardian's vote against a proposal. At `veto_threshold` distinct votes it is gone. Returns whether it was vetoed.
    pub fn veto(&mut self, caller: Principal, id: u64, now_ns: u64) -> Result<bool, String> {
        if !self.is_guardian(&caller) {
            return Err(format!("refused: {caller} is not a guardian"));
        }
        let needed = self.roles.veto_threshold;
        // audit R2, N-6: a proposal being executed still takes votes; at the threshold it is cancelled, and `execute` installs
        // nothing if the buyer has not been installed yet
        if let Some(r) = self.running_mut(id) {
            if r.committed() {
                return Err(format!("refused: proposal {id} is already being installed; a veto can no longer stop it"));
            }
            if r.proposal.vetoes.contains(&caller) {
                return Err(format!("refused: {caller} has already voted against proposal {id}"));
            }
            r.proposal.vetoes.push(caller);
            let votes = r.proposal.vetoes.len() as u8;
            let by = r.proposal.vetoes.clone();
            if votes >= needed {
                r.cancelled.get_or_insert_with(|| "vetoed".into());
            }
            self.record(Event::VetoVote { id, at_ns: now_ns, by: caller, votes, needed });
            if votes >= needed {
                self.record(Event::Vetoed { id, at_ns: now_ns, by });
                return Ok(true);
            }
            return Ok(false);
        }
        let i = self.index_of(id)?;
        let p = &mut self.pending[i];
        if p.vetoes.contains(&caller) {
            return Err(format!("refused: {caller} has already voted against proposal {id}"));
        }
        p.vetoes.push(caller);
        let votes = p.vetoes.len() as u8;
        self.record(Event::VetoVote { id, at_ns: now_ns, by: caller, votes, needed });
        if votes >= needed {
            let gone = self.pending.remove(i);
            self.record(Event::Vetoed { id, at_ns: now_ns, by: gone.vetoes });
            return Ok(true);
        }
        Ok(false)
    }

    /// Anyone, once the delay has passed and the proposal is not stale. The proposal leaves `pending` **before** the caller
    /// awaits the management canister, so two `execute` calls in flight cannot both perform it. A roles change is applied here,
    /// in full, since it touches nothing outside this state. A stale proposal is removed, and the reason is the error.
    pub fn take_ready(&mut self, id: u64, now_ns: u64) -> Result<Ready, String> {
        if self.running_mut(id).is_some() {
            return Err(format!("refused: proposal {id} is running"));
        }
        let i = self.index_of(id)?;
        let p = &self.pending[i];
        if now_ns < p.executable_ns {
            return Err(format!("refused: proposal {id} may run in {} s", (p.executable_ns - now_ns).div_ceil(NS)));
        }
        let stale = if p.action.touches_target() {
            (p.target_epoch != self.target_epoch)
                .then(|| format!("stale: another upgrade or controller change has executed since proposal {id} was made"))
        } else {
            (p.roles_epoch != self.roles_epoch).then(|| format!("stale: the roles have changed since proposal {id} was made"))
        };
        let p = self.pending.remove(i);
        if let Some(why) = stale {
            return Err(why);
        }
        match &p.action {
            Action::Roles { roles } => {
                // validated at proposal time under the same rules; validated again, because a rule may not be skipped
                roles.validate()?;
                self.roles = roles.clone();
                self.roles_epoch += 1;
                self.resume_votes.clear();
                self.resume_round_ns = None;
                // a pending proposal by someone no longer a proposer does not run; one being executed is cancelled (N-6)
                let proposers = self.roles.proposers.clone();
                self.pending.retain(|q| proposers.contains(&q.by));
                for r in self.running.iter_mut().flatten().filter(|r| !proposers.contains(&r.proposal.by) && !r.committed()) {
                    r.cancelled.get_or_insert_with(|| "its proposer is no longer a proposer".into());
                }
                self.record(Event::RolesChanged { id, at_ns: now_ns, roles: roles.clone() });
                Ok(Ready::RolesApplied(p))
            }
            _ => {
                self.target_epoch += 1;
                // audit R2, N-6: kept, visible, until `execute` finishes
                // a controllers change has a single await and no check after it: committed at once (audit R3, R3-C5)
                let committed = matches!(p.action, Action::Controllers { .. }).then_some(true);
                self.running.get_or_insert_with(Vec::new).push(Running { proposal: p.clone(), cancelled: None, since_ns: Some(now_ns),
                    committed });
                Ok(Ready::Target(p))
            }
        }
    }

    fn running_mut(&mut self, id: u64) -> Option<&mut Running> {
        self.running.as_mut()?.iter_mut().find(|r| r.proposal.id == id)
    }

    /// Whether a proposal being executed is still wanted: `Err` names what cancelled it while its buyer was being stopped.
    pub fn still_wanted(&mut self, id: u64) -> Result<(), String> {
        let proposers = self.roles.proposers.clone();
        let r = self.running_mut(id).ok_or_else(|| format!("proposal {id} is not running"))?;
        if !proposers.contains(&r.proposal.by) {
            r.cancelled.get_or_insert_with(|| "its proposer is no longer a proposer".into());
        }
        match &r.cancelled {
            Some(why) => Err(why.clone()),
            None => Ok(()),
        }
    }

    /// `execute` passed its last check for a proposal (its buyer is stopped and nothing cancelled it): from here it installs,
    /// and a veto or a withdrawal is refused rather than recorded (audit R3, R3-C5). Call it with [`State::still_wanted`], with
    /// no await between.
    pub fn commit(&mut self, id: u64) -> Result<(), String> {
        let r = self.running_mut(id).ok_or_else(|| format!("proposal {id} is not running"))?;
        r.committed = Some(true);
        Ok(())
    }

    /// Running entries left behind by an `execute` that never finished (audit R3, R3-C5): older than [`RUNNING_ORPHAN_NS`], or
    /// saved before `since_ns` existed. Each is removed and recorded as `Failed`; its epoch stays moved (the conservative side:
    /// other target proposals made before it must be made again). The ids cleared.
    pub fn clear_orphans(&mut self, by: Principal, now_ns: u64) -> Vec<u64> {
        let Some(v) = self.running.as_mut() else { return vec![] };
        let (gone, kept): (Vec<Running>, Vec<Running>) = std::mem::take(v).into_iter()
            .partition(|r| r.since_ns.is_none_or(|t| now_ns.saturating_sub(t) >= RUNNING_ORPHAN_NS));
        *v = kept;
        let ids: Vec<u64> = gone.iter().map(|r| r.proposal.id).collect();
        for id in &ids {
            self.record(Event::Failed { id: *id, at_ns: now_ns, by, why: format!("orphaned: its execute never finished (still \
                running after {} h); cleared by a guardian's start_target", RUNNING_ORPHAN_NS / 3600 / NS) });
        }
        ids
    }

    /// `execute` is over for a proposal: it leaves the running list.
    pub fn finish(&mut self, id: u64) -> Option<Proposal> {
        let v = self.running.as_mut()?;
        let i = v.iter().position(|r| r.proposal.id == id)?;
        Some(v.remove(i).proposal)
    }

    /// A target action taken by `take_ready` that did not happen (the buyer would not stop, or it was cancelled while it was
    /// stopping, so nothing was installed: audit D, L-2; audit R2, N-6). The epoch it moved comes back. It returns to pending
    /// only if nothing cancelled it and its proposer still is one, with its veto votes, and not runnable again for
    /// [`STOP_RETRY_NS`]: never twice at once, never after a veto. `Ok(true)`: pending again; `Ok(false)`: dropped.
    pub fn untake(&mut self, id: u64, now_ns: u64) -> Result<bool, String> {
        let wanted = self.still_wanted(id);
        let r = self.running.as_mut().and_then(|v| v.iter().position(|r| r.proposal.id == id).map(|i| v.remove(i)))
            .ok_or_else(|| format!("proposal {id} is not running"))?;
        let mut p = r.proposal;
        if p.action.touches_target() && self.target_epoch == p.target_epoch + 1 {
            self.target_epoch -= 1;
        }
        if wanted.is_err() {
            return Ok(false);
        }
        p.executable_ns = p.executable_ns.max(now_ns.saturating_add(STOP_RETRY_NS));
        self.pending.push(p);
        Ok(true)
    }

    /// Audit R2, N-6: any guardian may start the buyer (starting is safe: a paused buyer stays paused by its own flag, and the
    /// code is whatever is installed), except while an `execute` is stopping it for an upgrade. Orphaned running entries are
    /// cleared first (audit R3, R3-C5), so a trapped `execute` cannot refuse `start_target` for ever.
    pub fn may_start(&mut self, caller: Principal, now_ns: u64) -> Result<(), String> {
        if !self.is_guardian(&caller) {
            return Err(format!("refused: {caller} is not a guardian"));
        }
        self.clear_orphans(caller, now_ns);
        if let Some(r) = self.running.iter().flatten().next() {
            return Err(format!("refused: proposal {} is being executed; starting the buyer now would break its stop", r.proposal.id));
        }
        Ok(())
    }

    /// Any single guardian may stop the buyer: stopping is always safe.
    pub fn may_pause(&mut self, caller: Principal) -> Result<(), String> {
        if !self.is_guardian(&caller) {
            return Err(format!("refused: {caller} is not a guardian"));
        }
        self.resume_votes.clear();
        self.resume_round_ns = None;
        Ok(())
    }

    /// Audit D, L-3: whether an ingress message is worth accepting at all (the timelock pays for every one, and it holds only
    /// 2 T cycles and is blackholed). Proposers may propose and withdraw; guardians may veto, pause and resume; anyone may
    /// execute a pending proposal whose delay has passed (`id` is the call's decoded argument). Everything else would only be
    /// refused by the method itself. A cycles guard, not the access control: every method keeps its own checks.
    pub fn admits(&self, caller: Principal, method: &str, id: Option<u64>, now_ns: u64) -> bool {
        match method {
            "propose_upgrade" | "propose_controllers" | "propose_roles" | "withdraw" | "stage_chunk" | "unstage" | "propose_staged_upgrade" => {
                self.roles.proposers.contains(&caller)
            }
            "veto" | "pause_target" | "resume_target" | "start_target" => self.is_guardian(&caller),
            "execute" => id.is_some_and(|i| self.pending.iter().any(|p| p.id == i && now_ns >= p.executable_ns)),
            _ => false,
        }
    }

    /// Resuming takes `veto_threshold` guardians, so a single guardian cannot lift the others' pause. Returns whether enough
    /// have now voted.
    pub fn resume_vote(&mut self, caller: Principal, now_ns: u64) -> Result<bool, String> {
        if !self.is_guardian(&caller) {
            return Err(format!("refused: {caller} is not a guardian"));
        }
        // I-4: votes older than the round's life no longer count
        if self.resume_round_ns.is_some_and(|t| now_ns.saturating_sub(t) > RESUME_VOTE_TTL_NS) {
            self.resume_votes.clear();
            self.resume_round_ns = None;
        }
        if self.resume_votes.is_empty() {
            self.resume_round_ns = Some(now_ns);
        }
        if !self.resume_votes.contains(&caller) {
            self.resume_votes.push(caller);
        }
        let (votes, needed) = (self.resume_votes.len() as u8, self.roles.veto_threshold);
        self.record(Event::ResumeVote { at_ns: now_ns, by: caller, votes, needed });
        if votes >= needed {
            self.resume_votes.clear();
            self.resume_round_ns = None;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn record(&mut self, e: Event) {
        self.history.push(e);
        if self.history.len() > 1000 {
            self.history.remove(0);
        }
    }

    /// W14a: a proposer appends one piece to the module it is staging (started by its first piece). Answers the bytes staged so far and
    /// their SHA-256 (hex), for the proposer's script to check against the file it is sending.
    pub fn stage(&mut self, caller: Principal, chunk: &[u8], now_ns: u64) -> Result<(u64, String), String> {
        if !self.roles.proposers.contains(&caller) {
            return Err(format!("refused: {caller} is not a proposer"));
        }
        if chunk.is_empty() || chunk.len() > MAX_STAGE_CHUNK {
            return Err(format!("refused: a piece of {} bytes; 1 to {MAX_STAGE_CHUNK}", chunk.len()));
        }
        let staged = self.staged.get_or_insert_with(Vec::new);
        let i = match staged.iter().position(|s| s.by == caller) {
            Some(i) => i,
            None => {
                staged.push(Staged { by: caller, bytes: Vec::new(), since_ns: now_ns });
                staged.len() - 1
            }
        };
        let s = &mut staged[i];
        if s.bytes.len() + chunk.len() > MAX_CHUNKED_MODULE_BYTES {
            return Err(format!("refused: {} bytes staged with this piece; a module is at most {MAX_CHUNKED_MODULE_BYTES}", s.bytes.len() + chunk.len()));
        }
        s.bytes.extend_from_slice(chunk);
        Ok((s.bytes.len() as u64, sha256_hex(&s.bytes)))
    }

    /// W14a: a proposer drops what it staged.
    pub fn unstage(&mut self, caller: Principal) -> Result<(), String> {
        let staged = self.staged.get_or_insert_with(Vec::new);
        let before = staged.len();
        staged.retain(|s| s.by != caller);
        if staged.len() == before {
            return Err(format!("refused: {caller} has nothing staged"));
        }
        Ok(())
    }

    /// W14a: what `caller` has staged: its size and SHA-256 (hex).
    pub fn staged_of(&self, caller: Principal) -> Option<(u64, String)> {
        self.staged.iter().flatten().find(|s| s.by == caller).map(|s| (s.bytes.len() as u64, sha256_hex(&s.bytes)))
    }

    /// W14a: propose the staged module whole, as `propose` would a module sent in one message (the same checks, the same delay from
    /// now). `sha256_hex` must be the full module's hash as the proposer means it: a staging that lost or doubled a piece is refused,
    /// and stays staged. Once proposed, the staging is gone.
    pub fn propose_staged(&mut self, caller: Principal, sha256: &str, arg: Vec<u8>, now_ns: u64) -> Result<ProposalView, String> {
        let (_, have) = self.staged_of(caller).ok_or_else(|| format!("refused: {caller} has nothing staged"))?;
        if !have.eq_ignore_ascii_case(sha256.trim()) {
            return Err(format!("refused: the staged module's SHA-256 is {have}, not {}: stage it again (unstage first)", sha256.trim()));
        }
        let i = self.staged.iter().flatten().position(|s| s.by == caller).expect("found above");
        let wasm = self.staged.as_ref().expect("found above")[i].bytes.clone();
        let v = self.propose(caller, Action::Upgrade { wasm, arg }, now_ns)?;
        self.staged.as_mut().expect("found above").remove(i);
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: u64 = NS;
    fn p(n: u8) -> Principal {
        Principal::from_slice(&[n; 10])
    }
    /// proposers 20, 21; guardians 30, 31, 32; veto 2 of 3 — the smallest production shape
    fn roles() -> Roles {
        Roles { proposers: vec![p(20), p(21)], guardians: vec![p(30), p(31), p(32)], veto_threshold: 2 }
    }
    fn cfg() -> Config {
        Config { target: p(1), roles: roles(), delay_secs: MIN_DELAY_SECS }
    }
    fn wasm() -> Vec<u8> {
        let mut w = b"\0asm\x01\0\0\0".to_vec();
        w.extend_from_slice(&[7; 100]);
        w
    }
    fn up() -> Action {
        Action::Upgrade { wasm: wasm(), arg: vec![0x44, 0x49, 0x44, 0x4c] }
    }
    const AT: u64 = MIN_DELAY_SECS * NS;

    #[test]
    fn it_is_installed_only_with_a_delay_at_the_floor_and_sound_roles() {
        assert!(State::new(cfg()).is_ok());
        assert!(State::new(Config { delay_secs: MIN_DELAY_SECS - 1, ..cfg() }).unwrap_err().contains("below this build's floor"));
        assert!(State::new(Config { target: Principal::anonymous(), ..cfg() }).unwrap_err().contains("anonymous"));
        let bad = |r: Roles| State::new(Config { roles: r, ..cfg() }).unwrap_err();
        assert!(bad(Roles { proposers: vec![Principal::anonymous(), p(21)], ..roles() }).contains("anonymous"));
        assert!(bad(Roles { proposers: vec![], ..roles() }).contains("at least one proposer"));
        assert!(bad(Roles { guardians: vec![p(30), p(30), p(31)], ..roles() }).contains("listed twice"));
        assert!(bad(Roles { veto_threshold: 4, ..roles() }).contains("veto threshold of 4"));
        assert!(bad(Roles { proposers: vec![p(1); 6], ..roles() }).contains("at most 5"));
        #[cfg(not(feature = "rehearsal"))]
        {
            assert_eq!(MIN_DELAY_SECS, 72 * 3600, "a production build waits three days");
            assert!(bad(Roles { proposers: vec![p(20)], ..roles() }).contains("name two at least"));
            assert!(bad(Roles { guardians: vec![p(30), p(31)], veto_threshold: 1, ..roles() }).contains("fewer than three guardians"));
            assert!(bad(Roles { veto_threshold: 1, ..roles() }).contains("at least 2"), "one guardian alone could freeze the buyer");
            assert!(bad(Roles { veto_threshold: 3, ..roles() }).contains("at most 2"), "losing one guardian would lose the veto");
            assert!(bad(Roles { guardians: vec![p(20), p(31), p(32)], ..roles() }).contains("both a proposer and a guardian"));
        }
    }

    #[test]
    fn any_proposer_proposes_one_pending_each_and_the_hash_is_published() {
        let mut s = State::new(cfg()).unwrap();
        for who in [p(30), p(9), Principal::anonymous()] {
            assert!(s.propose(who, up(), 0).unwrap_err().contains("is not a proposer"), "a guardian cannot propose either");
        }
        let v = s.propose(p(20), up(), 5 * S).unwrap();
        assert_eq!(v.wasm_hash.as_deref(), Some(sha256_hex(&wasm()).as_str()));
        assert_eq!((v.id, v.by, v.executable_ns, v.arg_hex.as_deref()), (1, p(20), 5 * S + AT, Some("4449444c")));
        assert!(s.propose(p(20), up(), 6 * S).unwrap_err().contains("already has proposal 1 pending"));
        // one stolen proposer cannot occupy the only slot: the other proposes beside it
        assert_eq!(s.propose(p(21), Action::Roles { roles: roles() }, 6 * S).unwrap().id, 2);
        assert_eq!(s.pending.len(), 2);
    }

    #[test]
    fn a_proposer_withdraws_only_its_own_and_one_guardian_cannot_veto_alone() {
        let mut s = State::new(cfg()).unwrap();
        s.propose(p(20), up(), 0).unwrap();
        assert!(s.withdraw(p(21), 1, 1).unwrap_err().contains("not"), "a stolen proposer cannot clear another's proposal");
        assert!(s.veto(p(20), 1, 1).unwrap_err().contains("not a guardian"));
        assert!(!s.veto(p(30), 1, 1).unwrap(), "one vote of two");
        assert!(s.veto(p(30), 1, 2).unwrap_err().contains("already voted"), "the same guardian twice is still one vote");
        assert_eq!(s.pending.len(), 1, "a single guardian, stolen or not, cannot veto alone");
        assert!(s.veto(p(31), 1, 3).unwrap(), "the second guardian's vote vetoes it");
        assert!(s.pending.is_empty());
        assert!(matches!(s.history.last(), Some(Event::Vetoed { id: 1, by, .. }) if by == &vec![p(30), p(31)]));
        assert!(s.take_ready(1, u64::MAX).is_err(), "a vetoed proposal never runs");
        s.propose(p(20), up(), 10).unwrap();
        s.withdraw(p(20), 2, 11).unwrap();
        assert!(s.pending.is_empty());
    }

    #[test]
    fn nothing_runs_before_the_delay_anyone_runs_it_after_once_and_a_stale_one_fails() {
        let mut s = State::new(cfg()).unwrap();
        s.propose(p(20), up(), 0).unwrap();
        s.propose(p(21), up(), 0).unwrap();
        assert!(s.take_ready(1, AT - 1).unwrap_err().contains("may run in 1 s"));
        assert_eq!(s.pending.len(), 2, "an early attempt changes nothing");
        assert!(matches!(s.take_ready(1, AT).unwrap(), Ready::Target(q) if q.action == up() && q.id == 1));
        assert!(s.take_ready(1, AT).unwrap_err().contains("is running"), "taken once: a second execute in flight is refused");
        s.finish(1);
        assert!(s.take_ready(1, AT).unwrap_err().contains("not pending"), "and once finished, it is gone");
        // proposal 2 was made before proposal 1 ran: it must not land on top of it
        assert!(s.take_ready(2, AT).unwrap_err().contains("stale"));
        assert!(s.pending.is_empty(), "a stale proposal is cleared; proposing again means waiting again");
        let v = s.propose(p(21), up(), AT).unwrap();
        assert!(matches!(s.take_ready(v.id, AT + AT), Ok(Ready::Target(_))), "made after the last one ran: it runs");
    }

    #[test]
    fn roles_change_like_code_and_a_removed_proposer_stops_proposing() {
        let mut s = State::new(cfg()).unwrap();
        // p(20) is stolen: it proposes an upgrade; p(21) proposes roles without it
        s.propose(p(20), up(), 0).unwrap();
        let new = Roles { proposers: vec![p(21), p(22)], ..roles() };
        let r = s.propose(p(21), Action::Roles { roles: new.clone() }, 0).unwrap();
        assert_eq!(r.roles.as_ref(), Some(&new));
        assert!(s.withdraw(p(20), r.id, 1).is_err(), "the stolen proposer cannot withdraw the roles change");
        assert!(!s.veto(p(30), r.id, 1).unwrap(), "nor can one stolen guardian veto it alone");
        s.veto(p(31), 1, 2).unwrap();
        s.veto(p(32), 1, 2).unwrap(); // the honest guardians veto the stolen proposer's upgrade
        assert!(matches!(s.take_ready(r.id, AT).unwrap(), Ready::RolesApplied(_)));
        assert_eq!((s.roles.clone(), s.roles_epoch, s.target_epoch), (new, 1, 0), "roles changed; the target untouched");
        assert!(s.propose(p(20), up(), AT).unwrap_err().contains("is not a proposer"), "the removed key proposes nothing more");
        assert!(matches!(s.history.last(), Some(Event::RolesChanged { .. })));
        // a roles change proposed before that one ran is stale
        let mut s2 = State::new(cfg()).unwrap();
        let a = s2.propose(p(20), Action::Roles { roles: roles() }, 0).unwrap().id;
        let b = s2.propose(p(21), Action::Roles { roles: roles() }, 0).unwrap().id;
        s2.take_ready(a, AT).unwrap();
        assert!(s2.take_ready(b, AT).unwrap_err().contains("stale"));
        // a roles proposal must itself be sound
        #[cfg(not(feature = "rehearsal"))]
        assert!(State::new(cfg()).unwrap().propose(p(20), Action::Roles { roles: Roles { veto_threshold: 1, ..roles() } }, 0)
            .unwrap_err().contains("at least 2"));
        // and a pending proposal by a removed proposer does not survive its removal
        let mut s3 = State::new(cfg()).unwrap();
        let rid = s3.propose(p(21), Action::Roles { roles: Roles { proposers: vec![p(21), p(22)], ..roles() } }, 0).unwrap().id;
        s3.propose(p(20), Action::Controllers { list: vec![p(5)] }, 1).unwrap();
        s3.take_ready(rid, AT).unwrap();
        assert!(s3.pending.is_empty());
    }

    #[test]
    fn any_guardian_pauses_and_resuming_takes_the_threshold() {
        let mut s = State::new(cfg()).unwrap();
        assert!(s.may_pause(p(20)).is_err(), "a proposer cannot pause");
        s.may_pause(p(32)).unwrap();
        assert!(!s.resume_vote(p(32), 1).unwrap(), "one guardian cannot resume alone");
        assert!(!s.resume_vote(p(32), 2).unwrap(), "voting twice is one vote");
        assert!(s.resume_vote(p(30), 3).unwrap(), "two resume it");
        s.may_pause(p(31)).unwrap();
        assert!(!s.resume_vote(p(30), 4).unwrap(), "a new pause clears the old votes");
        assert!(s.resume_vote(p(20), 5).is_err());
    }

    #[test]
    fn a_proposal_must_be_something_the_ic_would_accept() {
        let mut s = State::new(cfg()).unwrap();
        let bad = Action::Upgrade { wasm: b"MZ\x90\0".to_vec(), arg: vec![] };
        assert!(s.propose(p(20), bad, 0).unwrap_err().contains("not a WebAssembly module"));
        let gz = Action::Upgrade { wasm: vec![0x1f, 0x8b, 0x08, 0, 0], arg: vec![] };
        assert!(s.propose(p(20), gz, 0).is_ok(), "gzip is accepted, as the IC accepts it");
        s.withdraw(p(20), 1, 0).unwrap();
        let huge = Action::Upgrade { wasm: [b"\0asm".to_vec(), vec![0; MAX_CHUNKED_MODULE_BYTES]].concat(), arg: vec![] };
        assert!(s.propose(p(20), huge, 0).unwrap_err().contains("a module is at most"));
        for (list, why) in [(vec![p(4); 11], "the IC allows 10"), (vec![Principal::anonymous()], "anyone can use"), (vec![p(4), p(4)], "listed twice")] {
            assert!(s.propose(p(20), Action::Controllers { list }, 0).unwrap_err().contains(why), "{why}");
        }
        let v = s.propose(p(20), Action::Controllers { list: vec![p(5)] }, 0).unwrap();
        assert_eq!((v.kind.as_str(), v.controllers, v.wasm_hash), ("controllers", Some(vec![p(5)]), None));
    }

    /// Audit R2, N-6: a state saved by the build before `running` existed (round 1) decodes into this one, pending proposals
    /// and all, with nothing running.
    #[test]
    fn a_state_saved_before_running_existed_decodes_with_nothing_running() {
        #[derive(CandidType, Serialize)]
        struct Round1 {
            target: Principal,
            delay_secs: u64,
            roles: Roles,
            pending: Vec<Proposal>,
            next_id: u64,
            target_epoch: u64,
            roles_epoch: u64,
            resume_votes: Vec<Principal>,
            history: Vec<Event>,
            resume_round_ns: Option<u64>,
        }
        let mut s = State::new(cfg()).unwrap();
        s.propose(p(20), up(), 0).unwrap();
        let old = Round1 { target: s.target, delay_secs: s.delay_secs, roles: s.roles.clone(), pending: s.pending.clone(), next_id: 2,
            target_epoch: 0, roles_epoch: 0, resume_votes: vec![], history: vec![], resume_round_ns: None };
        let bytes = candid::encode_args((Some(old),)).unwrap();
        let (back,): (Option<State>,) = candid::decode_args(&bytes).unwrap();
        let back = back.unwrap();
        assert_eq!((back.pending, back.running), (s.pending, None));
    }

    /// Audit D, I-4: a vote from an earlier round does not help resume after a later pause the timelock never saw.
    #[test]
    fn a_stale_resume_vote_expires_with_its_round() {
        let mut s = State::new(cfg()).unwrap();
        let g = |i| roles().guardians[i];
        assert!(!s.resume_vote(g(0), 0).unwrap(), "one vote, round begins");
        // a day later (a tripwire paused the buyer meanwhile; the timelock did not see it): one new vote is not enough
        assert!(!s.resume_vote(g(1), RESUME_VOTE_TTL_NS + 1).unwrap(), "the old vote expired with its round");
        assert_eq!(s.resume_votes, vec![g(1)]);
        assert!(s.resume_vote(g(2), RESUME_VOTE_TTL_NS + 2).unwrap(), "two votes in one round resume");
        assert!(s.resume_votes.is_empty() && s.resume_round_ns.is_none());
    }

    /// Audit D, L-3 and L-2: which ingress is worth paying for, and a proposal whose buyer did not stop goes back as it was.
    #[test]
    fn ingress_is_admitted_by_role_and_an_upgrade_that_did_not_happen_returns_to_pending() {
        let mut s = State::new(cfg()).unwrap();
        let (p0, g0, stranger) = (roles().proposers[0], roles().guardians[0], p(99));
        assert!(s.admits(p0, "propose_upgrade", None, 0) && !s.admits(stranger, "propose_upgrade", None, 0));
        assert!(s.admits(g0, "veto", None, 0) && s.admits(g0, "pause_target", None, 0) && !s.admits(p0, "resume_target", None, 0));
        assert!(!s.admits(stranger, "status", None, 0) && !s.admits(g0, "anything", None, 0));
        let v = s.propose(p0, up(), 0).unwrap();
        let ripe = v.executable_ns;
        assert!(!s.admits(stranger, "execute", Some(v.id), ripe - 1), "not before its delay");
        assert!(s.admits(stranger, "execute", Some(v.id), ripe), "anyone, after it");
        assert!(!s.admits(stranger, "execute", Some(v.id + 1), ripe) && !s.admits(stranger, "execute", None, ripe));
        let epoch = s.target_epoch;
        let Ready::Target(_) = s.take_ready(v.id, ripe).unwrap() else { panic!() };
        assert_eq!(s.target_epoch, epoch + 1);
        assert_eq!(s.untake(v.id, ripe), Ok(true));
        assert_eq!(s.target_epoch, epoch, "the epoch it moved is back");
        assert!(s.take_ready(v.id, ripe).unwrap_err().contains("may run in"), "not again at once (R2, N-6)");
        assert!(matches!(s.take_ready(v.id, ripe + STOP_RETRY_NS).unwrap(), Ready::Target(_)), "and it runs as before, once");
        assert!(s.admits(g0, "start_target", None, 0) && !s.admits(stranger, "start_target", None, 0));
    }

    /// Audit R2, N-6: a proposal being executed is visible. A veto or a withdrawal while the buyer is being stopped cancels it
    /// (nothing is installed, the epoch comes back, it is gone); so does a roles change that removes its proposer; a failed stop
    /// puts back only what is still wanted, and never runnable twice at once; `start_target` is a guardian's, never mid-execute.
    #[test]
    fn a_proposal_being_executed_can_still_be_vetoed_withdrawn_or_pruned_and_a_failed_stop_never_revives_it() {
        let g = |i: usize| roles().guardians[i];
        // a veto while the buyer is stopping
        let mut s = State::new(cfg()).unwrap();
        let v = s.propose(p(20), up(), 0).unwrap();
        s.take_ready(v.id, AT).unwrap();
        assert!(s.take_ready(v.id, AT).unwrap_err().contains("running"), "a second execute is refused");
        assert!(!s.admits(p(99), "execute", Some(v.id), AT), "and not admitted");
        assert!(!s.veto(g(0), v.id, AT).unwrap());
        assert!(s.veto(g(0), v.id, AT).unwrap_err().contains("already voted"));
        assert!(s.veto(g(1), v.id, AT).unwrap(), "vetoed while running");
        assert_eq!(s.still_wanted(v.id), Err("vetoed".into()));
        assert!(s.may_start(g(2), AT).unwrap_err().contains("being executed"));
        let epoch = s.target_epoch;
        assert_eq!(s.untake(v.id, AT), Ok(false), "dropped, not pending again");
        assert_eq!(s.target_epoch, epoch - 1);
        assert!(s.pending.is_empty() && s.running.as_ref().is_some_and(|r| r.is_empty()));
        assert!(s.may_start(g(2), AT).is_ok() && s.may_start(p(20), AT).is_err());
        // a withdrawal while the buyer is stopping: only its own proposer
        let v = s.propose(p(21), up(), AT).unwrap();
        s.take_ready(v.id, AT + AT).unwrap();
        assert!(s.withdraw(p(20), v.id, AT + AT).unwrap_err().contains("not"));
        s.withdraw(p(21), v.id, AT + AT).unwrap();
        assert!(s.still_wanted(v.id).unwrap_err().contains("withdrawn"));
        assert_eq!(s.untake(v.id, AT + AT), Ok(false));
        // a roles change that removes the proposer of one being executed
        let mut s = State::new(cfg()).unwrap();
        let up_id = s.propose(p(20), up(), 0).unwrap().id;
        let rid = s.propose(p(21), Action::Roles { roles: Roles { proposers: vec![p(21), p(22)], ..roles() } }, 0).unwrap().id;
        s.take_ready(up_id, AT).unwrap();
        s.take_ready(rid, AT).unwrap();
        assert!(s.still_wanted(up_id).unwrap_err().contains("no longer a proposer"));
        assert_eq!(s.untake(up_id, AT), Ok(false));
        // a finished execute leaves the running list
        let v = s.propose(p(21), up(), AT).unwrap();
        s.take_ready(v.id, AT + AT).unwrap();
        assert!(s.still_wanted(v.id).is_ok());
        assert_eq!(s.finish(v.id).map(|q| q.id), Some(v.id));
        assert!(s.still_wanted(v.id).is_err() && s.may_start(g(0), AT + AT).is_ok());
    }

    /// Audit R3, R3-C5 (a): an `execute` that trapped after an await leaves its running entry behind. It no longer refuses
    /// `start_target` for ever: after `RUNNING_ORPHAN_NS` a guardian's start clears it (recorded as `Failed`), a stranger's does
    /// not, and a younger entry still refuses the start as before. An entry saved before `since_ns` existed is an orphan.
    #[test]
    fn an_orphaned_running_entry_is_cleared_by_a_guardians_start_after_a_day() {
        let g = |i: usize| roles().guardians[i];
        let mut s = State::new(cfg()).unwrap();
        let v = s.propose(p(20), up(), 0).unwrap();
        s.take_ready(v.id, AT).unwrap(); // and the execute traps after its first await: nothing else happens
        assert!(s.may_start(g(0), AT + RUNNING_ORPHAN_NS - 1).unwrap_err().contains("being executed"), "younger: still refused");
        assert!(s.may_start(p(99), AT + RUNNING_ORPHAN_NS).unwrap_err().contains("not a guardian"));
        assert_eq!(s.running.as_ref().map(Vec::len), Some(1), "a stranger clears nothing");
        s.may_start(g(0), AT + RUNNING_ORPHAN_NS).unwrap();
        assert_eq!(s.running.as_ref().map(Vec::len), Some(0));
        assert!(matches!(s.history.last(), Some(Event::Failed { id, why, .. }) if *id == v.id && why.contains("orphaned")));
        assert!(s.take_ready(v.id, AT + RUNNING_ORPHAN_NS).is_err(), "it does not come back");
        // an entry saved before `since_ns` existed
        let v = s.propose(p(20), up(), AT + RUNNING_ORPHAN_NS).unwrap();
        let ripe = v.executable_ns;
        s.take_ready(v.id, ripe).unwrap();
        s.running.as_mut().unwrap()[0].since_ns = None;
        s.may_start(g(1), ripe).unwrap();
        assert!(s.running.as_ref().unwrap().is_empty());
    }

    /// Audit R3, R3-C5 (b): once `execute` has passed its last check (the buyer stopped, nothing cancelled it) the proposal is
    /// committed: a veto or a withdrawal is refused, not recorded as if it had stopped the install, and a roles change no longer
    /// marks it cancelled. A controllers change is committed when taken (its single await has no later check).
    #[test]
    fn a_veto_or_withdrawal_of_a_proposal_being_installed_is_refused_not_recorded() {
        let g = |i: usize| roles().guardians[i];
        let mut s = State::new(cfg()).unwrap();
        let v = s.propose(p(20), up(), 0).unwrap();
        s.take_ready(v.id, AT).unwrap();
        assert!(!s.veto(g(0), v.id, AT).unwrap(), "before the commit, a vote counts");
        s.still_wanted(v.id).unwrap();
        s.commit(v.id).unwrap();
        let events = s.history.len();
        assert!(s.veto(g(1), v.id, AT).unwrap_err().contains("being installed"));
        assert!(s.withdraw(p(20), v.id, AT).unwrap_err().contains("being installed"));
        assert_eq!(s.history.len(), events, "nothing recorded");
        assert_eq!(s.still_wanted(v.id), Ok(()));
        assert_eq!(s.finish(v.id).map(|q| q.id), Some(v.id));
        // a roles change removing its proposer while it installs: not marked cancelled
        let up_id = s.propose(p(20), up(), AT).unwrap().id;
        let rid = s.propose(p(21), Action::Roles { roles: Roles { proposers: vec![p(21), p(22)], ..roles() } }, AT).unwrap().id;
        let ripe = AT + cfg().delay_secs * NS;
        s.take_ready(up_id, ripe).unwrap();
        s.commit(up_id).unwrap();
        s.take_ready(rid, ripe).unwrap();
        assert!(s.running.as_ref().unwrap()[0].cancelled.is_none());
        s.finish(up_id);
        // a controllers change: committed when taken
        let c = s.propose(p(21), Action::Controllers { list: vec![p(5)] }, ripe).unwrap();
        let ripe = ripe + cfg().delay_secs * NS;
        s.take_ready(c.id, ripe).unwrap();
        assert!(s.veto(g(0), c.id, ripe).unwrap_err().contains("being installed"));
    }

    /// Audit R3, R3-C5: a state saved by round 3 (running entries without `since_ns`/`committed`) decodes; its running entry then
    /// counts as an orphan for a guardian's start.
    #[test]
    fn a_state_saved_by_round_three_decodes_and_its_running_entry_is_an_orphan() {
        #[derive(CandidType, Serialize)]
        struct Running3 {
            proposal: Proposal,
            cancelled: Option<String>,
        }
        #[derive(CandidType, Serialize)]
        struct Round3 {
            target: Principal,
            delay_secs: u64,
            roles: Roles,
            pending: Vec<Proposal>,
            next_id: u64,
            target_epoch: u64,
            roles_epoch: u64,
            resume_votes: Vec<Principal>,
            history: Vec<Event>,
            resume_round_ns: Option<u64>,
            running: Option<Vec<Running3>>,
        }
        let mut s = State::new(cfg()).unwrap();
        let v = s.propose(p(20), up(), 0).unwrap();
        s.take_ready(v.id, AT).unwrap();
        let r = s.running.clone().unwrap().remove(0);
        let old = Round3 { target: s.target, delay_secs: s.delay_secs, roles: s.roles.clone(), pending: vec![], next_id: 2,
            target_epoch: 1, roles_epoch: 0, resume_votes: vec![], history: vec![], resume_round_ns: None,
            running: Some(vec![Running3 { proposal: r.proposal.clone(), cancelled: None }]) };
        let bytes = candid::encode_args((Some(old),)).unwrap();
        let (back,): (Option<State>,) = candid::decode_args(&bytes).unwrap();
        let mut back = back.unwrap();
        let r = &back.running.as_ref().unwrap()[0];
        assert_eq!((r.proposal.id, r.since_ns, r.committed), (v.id, None, None));
        back.may_start(roles().guardians[0], AT).unwrap();
        assert!(back.running.unwrap().is_empty());
    }
}

/// W14a: the staged, chunked path keeps the single-message path's rules: a proposer only, the full module's hash published and checked,
/// the same delay from the proposal, and an install plan the IC accepts (1 MiB chunks, the full module's SHA-256).
#[cfg(test)]
mod chunked_tests {
    use super::*;

    fn p(n: u8) -> Principal {
        Principal::from_slice(&[n; 29])
    }
    fn cfg() -> Config {
        Config { target: p(1), roles: Roles { proposers: vec![p(20), p(21)], guardians: vec![p(30), p(31), p(32)], veto_threshold: 2 },
            delay_secs: MIN_DELAY_SECS }
    }
    /// A module over the one-message limit (the buyer: ≈ 1.8 MB gzipped, more raw).
    fn big() -> Vec<u8> {
        let mut w = b"\0asm\x01\0\0\0".to_vec();
        w.extend((0..2_300_000u32).map(|i| (i % 251) as u8));
        w
    }

    #[test]
    fn a_large_module_is_staged_in_pieces_and_proposed_whole_with_its_full_hash() {
        let mut s = State::new(cfg()).unwrap();
        let w = big();
        assert!(s.stage(p(30), &w[..10], 0).unwrap_err().contains("not a proposer"), "a guardian cannot stage");
        assert!(s.stage(p(20), &[], 0).unwrap_err().contains("a piece of 0 bytes"));
        assert!(s.stage(p(20), &vec![0; MAX_STAGE_CHUNK + 1], 0).unwrap_err().contains("a piece of"));
        let mut last = (0, String::new());
        for piece in w.chunks(MAX_STAGE_CHUNK) {
            last = s.stage(p(20), piece, 7).unwrap();
        }
        assert_eq!(last, (w.len() as u64, sha256_hex(&w)), "the running total and hash the proposer's script checks");
        // another proposer stages its own, apart
        s.stage(p(21), b"\0asm", 7).unwrap();
        assert_eq!(s.staged_of(p(21)).unwrap().0, 4);
        // the wrong hash (a piece lost or doubled) is refused and the staging kept
        assert!(s.propose_staged(p(20), &sha256_hex(b"other"), vec![], 9).unwrap_err().contains("stage it again"));
        assert!(s.staged_of(p(20)).is_some());
        // proposed whole: the published hash is the FULL module's, the delay runs from now, the staging is gone
        let v = s.propose_staged(p(20), &sha256_hex(&w).to_uppercase(), vec![0x44, 0x49], 9).unwrap();
        assert_eq!((v.wasm_hash.as_deref(), v.wasm_bytes), (Some(sha256_hex(&w).as_str()), Some(w.len() as u64)));
        assert_eq!(v.executable_ns, 9 + MIN_DELAY_SECS * 1_000_000_000);
        assert!(s.staged_of(p(20)).is_none());
        // one pending each, as ever: a second staged proposal waits (and stays staged)
        s.stage(p(20), b"\0asm", 10).unwrap();
        assert!(s.propose_staged(p(20), &sha256_hex(b"\0asm"), vec![], 11).unwrap_err().contains("already has proposal"));
        assert!(s.staged_of(p(20)).is_some());
        s.unstage(p(20)).unwrap();
        assert!(s.unstage(p(20)).is_err());
        // not a module: refused at the proposal, as in one message
        s.unstage(p(21)).unwrap();
        s.stage(p(21), b"MZ", 12).unwrap();
        assert!(s.propose_staged(p(21), &s.staged_of(p(21)).unwrap().1, vec![], 12).unwrap_err().contains("not a WebAssembly module"));
        // the ingress filter admits the staging methods to proposers only
        for m in ["stage_chunk", "unstage", "propose_staged_upgrade"] {
            assert!(s.admits(p(20), m, None, 0) && !s.admits(p(30), m, None, 0) && !s.admits(Principal::anonymous(), m, None, 0), "{m}");
        }
    }

    #[test]
    fn the_install_plan_is_one_call_for_a_small_module_and_the_chunk_store_for_a_large_one() {
        assert_eq!(install_plan(&[0; 1_000], &[1; 10]), InstallPlan::Single);
        assert_eq!(install_plan(&vec![0; SINGLE_INSTALL_BYTES - 4], &[1; 4]), InstallPlan::Single);
        let w = big();
        let InstallPlan::Chunked { pieces, hashes, module } = install_plan(&w, &[1; 4]) else { panic!("chunked") };
        assert_eq!(pieces.len(), w.len().div_ceil(IC_CHUNK_BYTES));
        assert!(pieces.iter().all(|(a, b)| b - a <= IC_CHUNK_BYTES && a < b), "the IC takes chunks of at most 1 MiB");
        assert_eq!(pieces.first().unwrap().0, 0);
        assert_eq!(pieces.last().unwrap().1, w.len());
        assert!(pieces.windows(2).all(|x| x[0].1 == x[1].0), "contiguous: the chunks concatenate to the module");
        let joined: Vec<u8> = pieces.iter().flat_map(|(a, b)| w[*a..*b].to_vec()).collect();
        assert_eq!(joined, w);
        for ((a, b), h) in pieces.iter().zip(&hashes) {
            assert_eq!(hex::encode(h), sha256_hex(&w[*a..*b]));
        }
        assert_eq!(hex::encode(module), sha256_hex(&w), "install_chunked_code names the FULL module's hash: the module hash after install");
        // a small module with a large argument goes through the chunk store too (the call carries both)
        assert!(matches!(install_plan(&[0; 1_000], &vec![0; SINGLE_INSTALL_BYTES]), InstallPlan::Chunked { .. }));
    }

    /// A state saved before W14a (no `staged`) decodes into this one.
    #[test]
    fn a_state_saved_before_staging_restores() {
        let s = State::new(cfg()).unwrap();
        #[derive(CandidType, Serialize)]
        struct Old {
            target: Principal,
            delay_secs: u64,
            roles: Roles,
            pending: Vec<Proposal>,
            next_id: u64,
            target_epoch: u64,
            roles_epoch: u64,
            resume_votes: Vec<Principal>,
            history: Vec<Event>,
            resume_round_ns: Option<u64>,
            running: Option<Vec<Running>>,
        }
        let old = Old { target: s.target, delay_secs: s.delay_secs, roles: s.roles.clone(), pending: vec![], next_id: 4, target_epoch: 1,
            roles_epoch: 0, resume_votes: vec![], history: vec![], resume_round_ns: None, running: None };
        let back: State = candid::decode_one(&candid::encode_one(Some(old)).unwrap()).map(|x: Option<State>| x.unwrap()).unwrap();
        assert_eq!((back.next_id, back.staged), (4, None));
    }
}
