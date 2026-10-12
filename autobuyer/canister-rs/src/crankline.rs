//! The crank line's rules, shared by the coins (`coins::run`), the pool desk (`pooldesk::run`) and the treasury cranks (`cranks`):
//! a step of theirs is never queued; it signs only on a free line ([`line_free`]), and its pass reads the world through [`CrankIo`]
//! (the clock, the pause, the drand relays, the raffle's published snapshots). What is left of the registry keeper's module
//! (`solcrank`, retired with rook_core: decision D-l, docs/v9/canister-only-solana.md §0).
use serde_json::Value;

use crate::solhome::{SolIo, SolLine};
use crate::soltx::Pubkey;

/// May a crank sign now (no outcall)? The line free, nobody queued for it, no incident, a fee payer that can pay.
pub fn line_free(l: &SolLine) -> Result<(), String> {
    if let Some(i) = &l.line.incident {
        return Err(format!("the line has an open incident ({i})"));
    }
    if l.busy() {
        return Err("the line is busy (a step of a flow, or ours, is out)".into());
    }
    if let Some((o, _)) = l.queue.first() {
        return Err(format!("the {o:?} side waits its turn on the line: money moves first"));
    }
    if l.line.error.as_deref().is_some_and(|e| e.contains("insufficient_funds")) {
        return Err("our Solana address cannot pay a fee (the line's error): fund it".into());
    }
    Ok(())
}

/// The world a crank pass needs: Solana (the lines, the reads, our key) and the drand relays.
#[allow(async_fn_in_trait)]
pub trait CrankIo: SolIo {
    fn now(&self) -> u64;
    fn paused(&self) -> bool;
    fn drand_relays(&self) -> Vec<String>;
    /// One relay's transformed answer for `round` (`drand::normalise`'s shape).
    async fn drand_fetch(&mut self, base: &str, round: u64) -> Result<Value, String>;
    /// Publish a raffle snapshot (before its commit is booked): kept and served by the `raffle_snapshot` query.
    fn raffle_publish(&mut self, s: crate::raffle::Snapshot) {
        crate::raffle::publish(s)
    }
    fn raffle_published(&self, listing: &Pubkey, round: u64) -> Option<crate::raffle::Snapshot> {
        crate::raffle::published(listing, round)
    }
}
