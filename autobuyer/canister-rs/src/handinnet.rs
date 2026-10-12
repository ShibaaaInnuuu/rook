//! Permissionless offer hand-ins, the canister's side (docs/v6/offer-handins.md §6): the state (stable, the upgrade tuple's
//! tenth element), the board a controller names, the one endpoint open to anyone (`submit_offer`: the bytes behind an
//! on-chain commitment, size- and rate-limited), and the queries. Nothing is decided here: `handin.rs` keeps the rules,
//! `handinflow.rs` drives them inside `frogflow`'s tick.

use std::cell::RefCell;

use crate::handin::{self, BoardView, HandinState, Rate, VerdictRec};

thread_local! {
    /// Saved across upgrades as the upgrade tuple's TENTH element.
    pub(crate) static HANDINS: RefCell<HandinState> = RefCell::new(HandinState::default());
    /// Accepted `submit_offer` calls per caller in the last hour. Heap: a limit that resets at an upgrade is still a limit.
    static RATE: RefCell<Rate> = RefCell::new(Rate::default());
}

fn only_controller() {
    let caller = ic_cdk::api::msg_caller();
    if !ic_cdk::api::is_controller(&caller) {
        ic_cdk::trap(&format!("refused: {caller} is not a controller of this canister."));
    }
}

fn hex32(s: &str) -> Result<[u8; 32], String> {
    hex::decode(s.trim().trim_start_matches("0x")).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| format!("{s:?} is not 32 bytes of hex"))
}

/// An `OfferBoard` this canister buys from. Added once and never removed: identity, like a sink. Refused unless a wrapper is
/// configured; that the board's wrapper is ours is checked at its first read.
#[ic_cdk::update]
pub fn set_offer_board(board: String) -> Result<String, String> {
    only_controller();
    crate::canister::not_ticking()?;
    if crate::canister::cfg().home.is_some() {
        return Err("refused: the offer board is OFF on the Solana home at launch (docs/v9/solana-migration.md §2.7)".into());
    }
    let a = crate::evmtx::parse_address(board.trim())?;
    if crate::canister::cfg().wrapper.is_none() {
        return Err("refused: no wrapper is configured; a board serves the wrapper's collection".into());
    }
    HANDINS.with(|h| h.borrow_mut().add_board(a))?;
    crate::canister::note(format!("offer board 0x{} added", hex::encode(a)));
    Ok(format!("board 0x{}: read at the next tick", hex::encode(a)))
}

/// Anyone: the bytes behind an on-chain submission — the PSBT (hex), the inputs the venue asks the canister to sign, and the
/// inscription's membership proof. Kept if their sha256 is an open submission the canister has read, else buffered until the
/// next read (docs/v6/offer-handins.md §3.3). Judged in full at a tick, never here; whoever delivers the bytes is not who is
/// paid — the board's record is.
#[ic_cdk::update]
pub fn submit_offer(psbt_hex: String, to_sign: Vec<u32>, proof_hex: Vec<String>) -> Result<String, String> {
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    let caller = ic_cdk::api::msg_caller();
    if caller == candid::Principal::anonymous() {
        return Err("refused: anonymous callers may not hand in offers".into());
    }
    if psbt_hex.len() > 2 * handin::MAX_PSBT_BYTES || proof_hex.len() > handin::MAX_PROOF_LEN || to_sign.len() > handin::MAX_TO_SIGN {
        return Err(format!("refused: at most {} bytes of PSBT, {} proof hashes and {} inputs", handin::MAX_PSBT_BYTES, handin::MAX_PROOF_LEN,
            handin::MAX_TO_SIGN));
    }
    let now = ic_cdk::api::time() / 1_000_000_000;
    if !RATE.with(|r| r.borrow_mut().allow(&caller.to_text(), now)) {
        return Err(format!("refused: at most {} hand-ins an hour per caller", handin::SUBMITS_PER_HOUR));
    }
    let psbt = hex::decode(psbt_hex.trim()).map_err(|e| format!("the PSBT is not hex: {e}"))?;
    let proof = proof_hex.iter().map(|p| hex32(p)).collect::<Result<Vec<_>, _>>()?;
    let (hash, matched) = HANDINS.with(|h| h.borrow_mut().intake(now, psbt, to_sign, proof))?;
    Ok(format!("bytes for {} {}", hex::encode(hash), if matched { "kept: their submission is open" } else { "buffered until the next read shows their submission" }))
}

/// The boards, their params, and how many open hand-ins each has (with bytes).
#[ic_cdk::query]
pub fn offer_board_state() -> Vec<BoardView> {
    HANDINS.with(|h| handin::views(&h.borrow()))
}

/// Every verdict: what was bought (and for which frog record), what was invalid (and why), what is gone, claimed or slashed.
#[ic_cdk::query]
pub fn handin_verdicts() -> Vec<VerdictRec> {
    HANDINS.with(|h| h.borrow().verdicts.clone())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_open_endpoint_checks_the_pause_the_caller_the_sizes_and_the_rate_and_configuration_is_the_controllers() {
        let src = include_str!("handinnet.rs");
        let needle = concat!("#[ic_cdk::", "update]");
        assert_eq!(src.matches(needle).count(), 2, "every update is counted here");
        let cfg = src.split(needle).nth(1).unwrap();
        assert!(cfg.trim_start().starts_with("pub fn set_offer_board") && cfg.contains("only_controller();"));
        let open: String = src.split(needle).nth(2).unwrap().chars().take(1_200).collect();
        assert!(open.trim_start().starts_with("pub fn submit_offer"));
        assert!(open.contains(&format!("{}_paused()?;", "not")), "refused while paused (H1)");
        assert!(open.contains("Principal::anonymous()"), "anonymous callers refused");
        assert!(open.contains("MAX_PSBT_BYTES") && open.contains("MAX_PROOF_LEN") && open.contains("MAX_TO_SIGN"), "sizes before decoding");
        assert!(open.contains("RATE.with"), "the per-caller rate before decoding");
        assert!(!open.contains("only_operator();"), "open to anyone: not the operator's");
    }
}
