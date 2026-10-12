//! The pool desk on the canister's surface: the IC world of the tick, and the views.
use candid::CandidType;
use serde::{Deserialize, Serialize};

use crate::chainlink::driver::DeskIo;
use crate::solkey::KeyPath;
use crate::soltx::{b58_encode, Pubkey};

use super::state::*;

pub(crate) struct PoolNet {
    sol: Option<crate::solhome::SolNet>,
}

crate::sol_io_via!(PoolNet, sol);

impl DeskIo for PoolNet {
    async fn key_of(&mut self, p: &KeyPath) -> Result<Pubkey, String> {
        crate::solkey::derive_key(p).await
    }
}

fn now_secs() -> u64 {
    ic_cdk::api::time() / 1_000_000_000
}

/// The desk's pass for the canister's timer and the runner's tick. Nothing without a Solana home.
pub(crate) async fn tick() -> Vec<String> {
    if crate::canister::cfg().home.is_none() {
        return vec![];
    }
    match crate::solhome::SolNet::of_install() {
        Ok(Some(sol)) => super::run::tick(&mut PoolNet { sol: Some(sol) }, now_secs()).await,
        Ok(None) => vec![],
        Err(e) => vec![format!("pool: waits: {e}")],
    }
}

/// One collection's desk as the operator and the site see it.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DeskView {
    pub coll: u32,
    pub pair: String,
    pub position: String,
    pub lo: i32,
    pub hi: i32,
    pub active: i32,
    pub pos_x: u64,
    pub pos_y: u64,
    pub native: u64,
    pub wsol: u64,
    pub wx: u64,
    pub fee_lamports: u64,
    pub fee_units: u64,
    pub swept: u64,
    pub arbs_last_hour: u32,
    pub arb_profit: u64,
    pub read_at: u64,
}

pub fn desks(now: u64) -> Vec<DeskView> {
    with(|s| {
        s.desks
            .iter()
            .map(|(coll, d)| DeskView {
                coll: *coll,
                pair: d.pair.map(|k| b58_encode(&k)).unwrap_or_default(),
                position: d.position.map(|k| b58_encode(&k)).unwrap_or_default(),
                lo: d.lo,
                hi: d.hi,
                active: d.active,
                pos_x: d.pos_x,
                pos_y: d.pos_y,
                native: d.native,
                wsol: d.wsol,
                wx: d.wx,
                fee_lamports: d.fee_lamports,
                fee_units: d.fee_units,
                swept: d.swept,
                arbs_last_hour: d.arbs.iter().filter(|t| **t + 3_600 > now).count() as u32,
                arb_profit: d.arb_profit,
                read_at: d.read_at,
            })
            .collect()
    })
}

/// The waits and the last landings (the monitor's view).
pub fn log() -> (Vec<Wait>, Vec<(String, u64, String)>) {
    with(|s| (s.waits.clone(), s.last.clone()))
}

// ---- the surface ----------------------------------------------------------------------------------------------------------------

/// Every collection's pool desk: its pair and position, its holdings, its fee income, its arbs (§10.1 `desk(coll)`).
#[ic_cdk::query]
pub fn pool_desks() -> Vec<crate::pooldesk::api::DeskView> {
    desks(now_secs())
}

/// The desk's waits, its last landings by kind, and the last steps that landed.
#[ic_cdk::query]
pub fn pool_log() -> (Vec<crate::pooldesk::state::Wait>, Vec<(String, u64, String)>, Vec<(u64, crate::pooldesk::step::PoolStep)>) {
    with(|s| (s.waits.clone(), s.last.clone(), s.history.clone()))
}

/// The operator switches the whole pool desk off (nothing signs; the books and the reads go on) or on again.
#[ic_cdk::update]
pub fn pool_set_off(off: bool) -> Result<String, String> {
    crate::canister::only_operator();
    with_mut(|s| s.off = off);
    Ok(format!("the pool desk is {}", if off { "off" } else { "on" }))
}
