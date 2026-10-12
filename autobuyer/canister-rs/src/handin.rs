//! Permissionless offer hand-ins (docs/v6/offer-handins.md): anyone commits to a marketplace offer on the collection's
//! `OfferBoard` (the sha256 of a seller-signed PSBT, the sats it asks of the float, the inscription) with a bond, delivers the
//! bytes to the canister, and earns a fixed reward once the purchase the offer described is six blocks deep in custody.
//!
//! Pure: no network, no keys, no clock of its own. Here: the board's ABI (selectors pinned against `cast sig`, the decoders of
//! `submissionOf`, `openIds` and `params` pinned against `cast abi-encode`), [`HandinState`] (boards, the submissions read at
//! the pin, the bytes delivered, the verdicts), the intake rules of `submit_offer` (size, hash match, the unmatched buffer,
//! the per-caller rate), the choice a wanted frog makes ([`HandinState::cheapest`]: the cheapest open hand-in with bytes,
//! under the cap, no verdict yet, with a purchase's latency left before its expiry), and which verdicts are due a claim or a
//! slash. The judging itself is `frogflow::accept`, exactly as for the operator's offers.

use sha2::{Digest, Sha256};

use crate::abi::{self, Arg};
use crate::frogbuy::{FPhase, FrogRec};
use crate::itemsink::Clock;

// ---- the ABI --------------------------------------------------------------------------------------------------------------------

pub const SUBMIT: [u8; 4] = [0x7f, 0x3b, 0x74, 0xcc];
pub const CLAIM: [u8; 4] = [0x37, 0x96, 0x07, 0xf5];
pub const SLASH: [u8; 4] = [0x45, 0xbc, 0x4d, 0x10];
pub const REFUND: [u8; 4] = [0x27, 0x8e, 0xcd, 0xe1];
pub const OPEN_IDS: [u8; 4] = [0xc3, 0xf9, 0x5d, 0xe6];
pub const OPEN_COUNT: [u8; 4] = [0xcf, 0xaf, 0x38, 0x0e];
pub const SUBMISSION_OF: [u8; 4] = [0x74, 0x50, 0x5d, 0x71];
pub const PARAMS: [u8; 4] = [0xcf, 0xf0, 0xab, 0x96];
pub const WRAPPER: [u8; 4] = [0xac, 0x21, 0x0c, 0xc7];
pub const SLASHABLE: [u8; 4] = [0x4b, 0xff, 0x9a, 0x80];
pub const EXPIRED: [u8; 4] = [0xba, 0x06, 0x5e, 0x1f];

/// `OfferBoard.Status`.
pub const STATUS_OPEN: u8 = 1;
pub const STATUS_CLAIMED: u8 = 2;
pub const STATUS_SLASHED: u8 = 3;
pub const STATUS_REFUNDED: u8 = 4;

// ---- the limits ------------------------------------------------------------------------------------------------------------------

/// A marketplace PSBT for one inscription is a few hundred bytes; the largest seen in rehearsals was under 2 KB.
pub const MAX_PSBT_BYTES: usize = 8_192;
pub const MAX_PROOF_LEN: usize = 32;
pub const MAX_TO_SIGN: usize = 64;
/// Bytes delivered before the canister has read their submission: kept this many, first in first out, for this long.
pub const MAX_UNMATCHED: usize = 32;
pub const UNMATCHED_TTL_SECS: u64 = 2 * 60 * 60;
/// Accepted `submit_offer` calls per caller per hour.
pub const SUBMITS_PER_HOUR: usize = 6;
/// The most open submissions read from a board (the board's own `maxOpen` is meant to be at most this).
pub const MAX_OPEN_READ: u64 = 64;
/// New submissions decoded per tick; the rest next tick.
pub const MAX_SUBMISSIONS_READ_PER_TICK: usize = 16;

/// `offerHash`: the sha256 of the PSBT bytes, what the submitter committed to on chain.
pub fn offer_hash(psbt: &[u8]) -> [u8; 32] {
    Sha256::digest(psbt).into()
}

// ---- encoders and decoders --------------------------------------------------------------------------------------------------------

pub fn call_uint(sel: [u8; 4], v: u128) -> Vec<u8> {
    abi::encode_call(sel, &[Arg::Word(abi::uint_word(v))])
}

pub fn open_ids(start: u64, max: u64) -> Vec<u8> {
    abi::encode_call(OPEN_IDS, &[Arg::Word(abi::uint_word(start as u128)), Arg::Word(abi::uint_word(max as u128))])
}

pub fn claim(id: u64) -> Vec<u8> {
    call_uint(CLAIM, id as u128)
}

pub fn slash(id: u64) -> Vec<u8> {
    call_uint(SLASH, id as u128)
}

/// `params()`: the board's immutables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct Params {
    pub bond_wei: u128,
    pub reward_wei: u128,
    pub slash_delay: u64,
    pub expiry: u64,
    pub reward_day_cap_wei: u128,
    pub max_open: u64,
}

pub fn decode_params(b: &[u8]) -> Result<Params, String> {
    Ok(Params {
        bond_wei: abi::uint_at(b, 0, 0, 128)?,
        reward_wei: abi::uint_at(b, 0, 1, 128)?,
        slash_delay: abi::uint_at(b, 0, 2, 64)? as u64,
        expiry: abi::uint_at(b, 0, 3, 64)? as u64,
        reward_day_cap_wei: abi::uint_at(b, 0, 4, 128)?,
        max_open: abi::uint_at(b, 0, 5, 64)? as u64,
    })
}

/// `submissionOf(id)`, as the board returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submission {
    pub submitter: [u8; 20],
    pub at: u64,
    pub at_block: u64,
    pub status: u8,
    pub bond_wei: u128,
    pub ask_sats: u64,
    pub hash: [u8; 32],
    pub inscription: String,
}

/// A returned struct with a dynamic member: one head word (the tuple's offset), then the tuple, whose string offset is
/// relative to the tuple's own start.
pub fn decode_submission(b: &[u8]) -> Result<Submission, String> {
    let base = abi::uint_at(b, 0, 0, 32)? as usize;
    let inscription = String::from_utf8(abi::bytes_at(b, base, 7)?).map_err(|_| "an inscription id that is not text")?;
    Ok(Submission {
        submitter: abi::addr_at(b, base, 0)?,
        at: abi::uint_at(b, base, 1, 64)? as u64,
        at_block: abi::uint_at(b, base, 2, 64)? as u64,
        status: abi::uint_at(b, base, 3, 8)? as u8,
        bond_wei: abi::uint_at(b, base, 4, 128)?,
        ask_sats: abi::uint_at(b, base, 5, 64)? as u64,
        hash: abi::word_at(b, base, 6)?.try_into().expect("32 bytes"),
        inscription,
    })
}

/// `openIds(start, max)`: a `uint256[]`.
pub fn decode_ids(b: &[u8]) -> Result<Vec<u64>, String> {
    let base = abi::uint_at(b, 0, 0, 32)? as usize;
    let n = abi::uint_at(b, base, 0, 32)? as usize;
    if n > MAX_OPEN_READ as usize {
        return Err(format!("{n} open ids, more than the {MAX_OPEN_READ} read"));
    }
    (0..n).map(|i| abi::uint_at(b, base, 1 + i, 64).map(|v| v as u64)).collect()
}

// ---- the state --------------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct BoardRec {
    pub board: [u8; 20],
    /// Read once, at the first tick: the board's immutables.
    pub params: Option<Params>,
    /// Why this board is never read (its wrapper is not ours), when that was found.
    pub refused: Option<String>,
}

/// One submission as read at a pin. `open` false once a read no longer lists it (claimed, slashed or refunded on chain).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct Seen {
    pub board: [u8; 20],
    pub id: u64,
    pub hash: [u8; 32],
    pub submitter: [u8; 20],
    pub at: u64,
    pub at_block: u64,
    pub ask_sats: u64,
    pub inscription: String,
    pub open: bool,
}

/// The bytes delivered for a hash.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct Bytes {
    pub hash: [u8; 32],
    pub psbt: Vec<u8>,
    pub to_sign: Vec<u32>,
    pub proof: Vec<[u8; 32]>,
    pub received_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub enum Verdict {
    /// Judged and signed for frog record `frog`; the claim is due once that frog is held.
    Bought { frog: u64 },
    /// A fault in the bytes: slashed once the board allows.
    Invalid { why: String },
    /// Not fillable, through no fault in the bytes (sold elsewhere, under the policy minimum, our purchase never confirmed):
    /// never tried again; the submitter takes the bond back at expiry.
    Gone { why: String },
    Claimed,
    Slashed,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct VerdictRec {
    pub board: [u8; 20],
    pub id: u64,
    pub verdict: Verdict,
    pub at: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct HandinState {
    pub boards: Vec<BoardRec>,
    pub seen: Vec<Seen>,
    /// Bytes matched to an open submission, one set per hash.
    pub bytes: Vec<Bytes>,
    /// Bytes no read has matched yet.
    pub unmatched: Vec<Bytes>,
    pub verdicts: Vec<VerdictRec>,
}

/// A hand-in ready to be offered to a wanted frog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub board: [u8; 20],
    pub id: u64,
    pub ask_sats: u64,
    pub inscription: String,
    pub psbt: Vec<u8>,
    pub to_sign: Vec<usize>,
    pub proof: Vec<[u8; 32]>,
}

impl HandinState {
    /// Any board configured (the buyer's `inspect_message` admits `submit_offer` only then: audit D, L-3).
    pub fn has_boards(&self) -> bool {
        !self.boards.is_empty()
    }

    pub fn add_board(&mut self, board: [u8; 20]) -> Result<(), String> {
        if self.boards.iter().any(|b| b.board == board) {
            return Ok(());
        }
        self.boards.push(BoardRec { board, params: None, refused: None });
        Ok(())
    }

    pub fn board(&self, a: &[u8; 20]) -> Option<&BoardRec> {
        self.boards.iter().find(|b| b.board == *a)
    }

    pub fn board_mut(&mut self, a: &[u8; 20]) -> Option<&mut BoardRec> {
        self.boards.iter_mut().find(|b| b.board == *a)
    }

    pub fn seen(&self, board: &[u8; 20], id: u64) -> Option<&Seen> {
        self.seen.iter().find(|s| s.board == *board && s.id == id)
    }

    pub fn bytes_for(&self, hash: &[u8; 32]) -> Option<&Bytes> {
        self.bytes.iter().find(|b| b.hash == *hash)
    }

    pub fn verdict_of(&self, board: &[u8; 20], id: u64) -> Option<&Verdict> {
        self.verdicts.iter().find(|v| v.board == *board && v.id == id).map(|v| &v.verdict)
    }

    pub fn set_verdict(&mut self, board: [u8; 20], id: u64, verdict: Verdict, now: u64) {
        match self.verdicts.iter_mut().find(|v| v.board == board && v.id == id) {
            Some(v) => {
                v.verdict = verdict;
                v.at = now;
            }
            None => self.verdicts.push(VerdictRec { board, id, verdict, at: now }),
        }
    }

    /// `submit_offer`'s rules: sizes, then the hash; bytes for an open submission are kept (the newest set for a hash wins),
    /// anything else goes to the bounded unmatched buffer. Returns the hash and whether it matched.
    pub fn intake(&mut self, now: u64, psbt: Vec<u8>, to_sign: Vec<u32>, proof: Vec<[u8; 32]>) -> Result<([u8; 32], bool), String> {
        if psbt.is_empty() || psbt.len() > MAX_PSBT_BYTES {
            return Err(format!("refused: a PSBT of {} bytes (at most {MAX_PSBT_BYTES})", psbt.len()));
        }
        if proof.len() > MAX_PROOF_LEN || to_sign.len() > MAX_TO_SIGN {
            return Err(format!("refused: at most {MAX_PROOF_LEN} proof hashes and {MAX_TO_SIGN} inputs to sign"));
        }
        if !psbt.starts_with(b"psbt\xff") {
            return Err("refused: not a PSBT".into());
        }
        let hash = offer_hash(&psbt);
        let b = Bytes { hash, psbt, to_sign, proof, received_at: now };
        if self.seen.iter().any(|s| s.open && s.hash == hash) {
            self.bytes.retain(|x| x.hash != hash);
            self.bytes.push(b);
            return Ok((hash, true));
        }
        self.prune_unmatched(now);
        self.unmatched.retain(|x| x.hash != hash);
        if self.unmatched.len() >= MAX_UNMATCHED {
            self.unmatched.remove(0);
        }
        self.unmatched.push(b);
        Ok((hash, false))
    }

    fn prune_unmatched(&mut self, now: u64) {
        self.unmatched.retain(|b| now.saturating_sub(b.received_at) < UNMATCHED_TTL_SECS);
    }

    /// One board read at a pin: `open` is every open id, `new` the submissions decoded this tick for ids not seen before.
    /// Submissions no longer open are closed (their bytes dropped); unmatched bytes whose hash is now open are matched; an
    /// open submission whose bytes have not arrived within the board's `slashDelay` is judged invalid.
    pub fn reconcile(&mut self, board: [u8; 20], open: &[u64], new: Vec<Submission>, ids: &[u64], now: u64) {
        for (id, s) in ids.iter().zip(new) {
            if self.seen(&board, *id).is_none() {
                self.seen.push(Seen { board, id: *id, hash: s.hash, submitter: s.submitter, at: s.at, at_block: s.at_block,
                    ask_sats: s.ask_sats, inscription: s.inscription, open: s.status == STATUS_OPEN });
            }
        }
        for s in self.seen.iter_mut().filter(|s| s.board == board && s.open) {
            if !open.contains(&s.id) {
                s.open = false;
            }
        }
        let open_hashes: Vec<[u8; 32]> = self.seen.iter().filter(|s| s.open).map(|s| s.hash).collect();
        self.bytes.retain(|b| open_hashes.contains(&b.hash));
        self.prune_unmatched(now);
        let (matched, rest): (Vec<Bytes>, Vec<Bytes>) = self.unmatched.drain(..).partition(|b| open_hashes.contains(&b.hash));
        self.unmatched = rest;
        for b in matched {
            self.bytes.retain(|x| x.hash != b.hash);
            self.bytes.push(b);
        }
        // finished records of closed submissions are forgotten once they are long closed; verdicts stay for the page
        let delay = self.board(&board).and_then(|b| b.params).map_or(u64::MAX, |p| p.slash_delay);
        let late: Vec<(u64, [u8; 32])> = self.seen.iter()
            .filter(|s| s.board == board && s.open && !open_hashes.is_empty() && self.bytes_for(&s.hash).is_none()
                && self.verdict_of(&board, s.id).is_none() && now >= s.at.saturating_add(delay))
            .map(|s| (s.id, s.hash)).collect();
        for (id, _) in late {
            self.set_verdict(board, id, Verdict::Invalid { why: "its bytes never arrived".into() }, now);
        }
    }

    /// Whether any open hand-in with bytes and no verdict exists: whether a wanted frog should read the rate and the clock.
    pub fn any_candidate(&self) -> bool {
        self.seen.iter().any(|s| s.open && self.bytes_for(&s.hash).is_some() && self.verdict_of(&s.board, s.id).is_none())
    }

    /// The cheapest open hand-in with bytes, no verdict, `ask_sats ≤ cap_sats`, and more than `latency` left on the nearer
    /// clock before its expiry. Ties by age (the lower id first).
    pub fn cheapest(&self, cap_sats: u64, clock: &Clock, latency: u64) -> Option<Candidate> {
        let mut best: Option<&Seen> = None;
        for s in self.seen.iter().filter(|s| s.open) {
            if s.ask_sats > cap_sats || self.verdict_of(&s.board, s.id).is_some() || self.bytes_for(&s.hash).is_none() {
                continue;
            }
            let Some(p) = self.board(&s.board).and_then(|b| b.params) else { continue };
            let left = crate::itemsink::secs_left(s.at + p.expiry, s.at_block + p.expiry / crate::frogbuy::PARENT_BLOCK_SECONDS, clock);
            if left <= latency {
                continue;
            }
            if best.is_none_or(|b| (s.ask_sats, s.id) < (b.ask_sats, b.id)) {
                best = Some(s);
            }
        }
        let s = best?;
        let b = self.bytes_for(&s.hash)?;
        Some(Candidate { board: s.board, id: s.id, ask_sats: s.ask_sats, inscription: s.inscription.clone(), psbt: b.psbt.clone(),
            to_sign: b.to_sign.iter().map(|i| *i as usize).collect(), proof: b.proof.clone() })
    }

    /// Bought hand-ins whose frog record ended without the frog in custody: `Gone`, no reward.
    pub fn settle_failed(&mut self, frogs: &[FrogRec], now: u64) {
        let gone: Vec<([u8; 20], u64, String)> = self.verdicts.iter().filter_map(|v| {
            let Verdict::Bought { frog } = v.verdict else { return None };
            let f = frogs.iter().find(|f| f.id == frog)?;
            match &f.phase {
                FPhase::Failed { why } => Some((v.board, v.id, format!("the purchase failed: {why}"))),
                FPhase::Stranded { why, .. } => Some((v.board, v.id, format!("the purchase stranded: {why}"))),
                _ => None,
            }
        }).collect();
        for (board, id, why) in gone {
            self.set_verdict(board, id, Verdict::Gone { why }, now);
        }
    }

    /// Claims due: bought hand-ins whose frog is held (or minted, settled or a stray: held first), still open on the board.
    pub fn claims_due(&self, frogs: &[FrogRec]) -> Vec<([u8; 20], u64, u64)> {
        self.verdicts.iter().filter_map(|v| {
            let Verdict::Bought { frog } = v.verdict else { return None };
            let f = frogs.iter().find(|f| f.id == frog)?;
            let held = matches!(f.phase, FPhase::Held { .. } | FPhase::Minted { .. } | FPhase::Settled { .. } | FPhase::Stray { .. });
            let open = self.seen(&v.board, v.id).is_some_and(|s| s.open);
            (held && open).then_some((v.board, v.id, frog))
        }).collect()
    }

    /// Slashes due: invalid hand-ins still open on the board (the board's `slashable` is read before the send).
    pub fn slashes_due(&self) -> Vec<([u8; 20], u64)> {
        self.verdicts.iter().filter(|v| matches!(v.verdict, Verdict::Invalid { .. }) && self.seen(&v.board, v.id).is_some_and(|s| s.open))
            .map(|v| (v.board, v.id)).collect()
    }
}

// ---- the per-caller rate -------------------------------------------------------------------------------------------------------------

/// The most callers the rate table keeps; the oldest go first.
pub const MAX_RATE_ENTRIES: usize = 1_000;

/// Accepted `submit_offer` calls per caller in a sliding hour. Heap: a limit that resets at an upgrade is a limit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rate {
    entries: Vec<(String, Vec<u64>)>,
}

impl Rate {
    pub fn allow(&mut self, who: &str, now: u64) -> bool {
        let e = match self.entries.iter_mut().find(|(w, _)| w == who) {
            Some(e) => e,
            None => {
                self.entries.push((who.to_string(), Vec::new()));
                self.entries.last_mut().expect("just pushed")
            }
        };
        e.1.retain(|t| now.saturating_sub(*t) < 3_600);
        if e.1.len() >= SUBMITS_PER_HOUR {
            return false;
        }
        e.1.push(now);
        // audit D, L-3: forget EVERY caller idle for an hour (principals are free, so the table would otherwise grow by one per
        // fresh caller and every call would scan it), and never keep more than MAX_RATE_ENTRIES
        for (_, ts) in self.entries.iter_mut() {
            ts.retain(|t| now.saturating_sub(*t) < 3_600);
        }
        self.entries.retain(|(_, ts)| !ts.is_empty());
        if self.entries.len() > MAX_RATE_ENTRIES {
            let drop = self.entries.len() - MAX_RATE_ENTRIES;
            self.entries.drain(..drop);
        }
        true
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ---- the page -----------------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct BoardView {
    pub board: String,
    pub params: Option<Params>,
    pub refused: Option<String>,
    pub open: u64,
    pub with_bytes: u64,
    pub unmatched: u64,
}

pub fn views(hs: &HandinState) -> Vec<BoardView> {
    hs.boards.iter().map(|b| BoardView {
        board: format!("0x{}", hex::encode(b.board)),
        params: b.params,
        refused: b.refused.clone(),
        open: hs.seen.iter().filter(|s| s.board == b.board && s.open).count() as u64,
        with_bytes: hs.seen.iter().filter(|s| s.board == b.board && s.open && hs.bytes_for(&s.hash).is_some()).count() as u64,
        unmatched: hs.unmatched.len() as u64,
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOARD: [u8; 20] = [0x0b; 20];
    const T0: u64 = 1_800_000_000;
    const B0: u64 = 26_000_000;

    fn params() -> Params {
        Params { bond_wei: 10_000_000_000_000_000, reward_wei: 10_000_000_000_000_000, slash_delay: 3_600, expiry: 86_400,
            reward_day_cap_wei: 200_000_000_000_000_000, max_open: 64 }
    }

    fn sub(hash: u8, ask: u64) -> Submission {
        Submission { submitter: [0x11; 20], at: T0, at_block: B0, status: STATUS_OPEN, bond_wei: params().bond_wei, ask_sats: ask,
            hash: [hash; 32], inscription: format!("{}i0", "ab".repeat(32)) }
    }

    fn psbt(tag: u8) -> Vec<u8> {
        let mut b = b"psbt\xff".to_vec();
        b.extend([tag; 40]);
        b
    }

    fn state() -> HandinState {
        let mut hs = HandinState::default();
        hs.add_board(BOARD).unwrap();
        hs.board_mut(&BOARD).unwrap().params = Some(params());
        hs
    }

    #[test]
    fn the_selectors_are_cast_sig_of_the_board_abi() {
        let sig = |s: &str| -> [u8; 4] { crate::evmtx::keccak(s.as_bytes())[..4].try_into().unwrap() };
        assert_eq!(SUBMIT, sig("submit(bytes32,uint256,string)"));
        assert_eq!(CLAIM, sig("claim(uint256)"));
        assert_eq!(SLASH, sig("slash(uint256)"));
        assert_eq!(REFUND, sig("refund(uint256)"));
        assert_eq!(OPEN_IDS, sig("openIds(uint256,uint256)"));
        assert_eq!(OPEN_COUNT, sig("openCount()"));
        assert_eq!(SUBMISSION_OF, sig("submissionOf(uint256)"));
        assert_eq!(PARAMS, sig("params()"));
        assert_eq!(WRAPPER, sig("wrapper()"));
        assert_eq!(SLASHABLE, sig("slashable(uint256)"));
        assert_eq!(EXPIRED, sig("expired(uint256)"));
        // `cast calldata "claim(uint256)" 7`
        assert_eq!(hex::encode(claim(7)), "379607f50000000000000000000000000000000000000000000000000000000000000007");
        assert_eq!(hex::encode(open_ids(0, 64)), "c3f95de600000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000040");
    }

    #[test]
    fn a_submission_and_the_params_decode_from_the_bytes_the_board_returns() {
        // test/handin/OfferBoard.t.sol, `test_submissionOfEncodesAsTheCanisterDecodes`: the same hex
        let b = hex::decode("00000000000000000000000000000000000000000000000000000000000000200000000000000000000000001111111111111111111111111111111111111111000000000000000000000000000000000000000000000000000000006b49d20000000000000000000000000000000000000000000000000000000000018cba800000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000002386f26fc1000000000000000000000000000000000000000000000000000000000000000dc758222222222222222222222222222222222222222222222222222222222222222200000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000042366662393736616234396463656330313766316532303165383433393539383332303461653161376332616266376365643061383564363932653434323739396930000000000000000000000000000000000000000000000000000000000000").unwrap();
        let s = decode_submission(&b).unwrap();
        assert_eq!(s, Submission { submitter: [0x11; 20], at: T0, at_block: B0, status: STATUS_OPEN, bond_wei: 10_000_000_000_000_000,
            ask_sats: 903_000, hash: [0x22; 32], inscription: "6fb976ab49dcec017f1e201e84395983204ae1a7c2abf7ced0a85d692e442799i0".into() });
        let p = hex::decode("000000000000000000000000000000000000000000000000002386f26fc10000000000000000000000000000000000000000000000000000002386f26fc100000000000000000000000000000000000000000000000000000000000000000e10000000000000000000000000000000000000000000000000000000000001518000000000000000000000000000000000000000000000000002c68af0bb1400000000000000000000000000000000000000000000000000000000000000000040").unwrap();
        assert_eq!(decode_params(&p).unwrap(), params());
        // `cast abi-encode "f(uint256[])" "[7,3,11]"`
        let ids = hex::decode("0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000300000000000000000000000000000000000000000000000000000000000000070000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000000b").unwrap();
        assert_eq!(decode_ids(&ids).unwrap(), vec![7, 3, 11]);
        assert!(decode_submission(&b[..100]).is_err());
    }

    #[test]
    fn intake_bounds_sizes_matches_open_hashes_and_buffers_the_rest() {
        let mut hs = state();
        assert!(hs.intake(T0, vec![], vec![], vec![]).unwrap_err().contains("bytes"));
        assert!(hs.intake(T0, vec![0u8; MAX_PSBT_BYTES + 1], vec![], vec![]).unwrap_err().contains("bytes"));
        assert!(hs.intake(T0, psbt(1), vec![], vec![[0; 32]; MAX_PROOF_LEN + 1]).unwrap_err().contains("proof"));
        assert!(hs.intake(T0, vec![1, 2, 3, 4, 5, 6], vec![], vec![]).unwrap_err().contains("not a PSBT"));
        // nothing open: unmatched
        let (h, matched) = hs.intake(T0, psbt(1), vec![0], vec![]).unwrap();
        assert!(!matched);
        assert_eq!(h, offer_hash(&psbt(1)));
        assert_eq!((hs.unmatched.len(), hs.bytes.len()), (1, 0));
        // the board now lists it: the read matches the buffered bytes
        let mut s = sub(1, 900_000);
        s.hash = h;
        hs.reconcile(BOARD, &[0], vec![s], &[0], T0 + 60);
        assert_eq!((hs.unmatched.len(), hs.bytes.len()), (0, 1));
        assert!(hs.any_candidate());
        // bytes for an open hash go straight in, and the newest set wins
        let (_, matched) = hs.intake(T0 + 61, psbt(1), vec![0, 1], vec![]).unwrap();
        assert!(matched);
        assert_eq!(hs.bytes.len(), 1);
        assert_eq!(hs.bytes[0].to_sign, vec![0, 1]);
        // the unmatched buffer is bounded, first in first out, and pruned by age
        for i in 0..(MAX_UNMATCHED as u8 + 5) {
            hs.intake(T0 + 100, psbt(10 + i), vec![], vec![]).unwrap();
        }
        assert_eq!(hs.unmatched.len(), MAX_UNMATCHED);
        assert_eq!(hs.unmatched[0].hash, offer_hash(&psbt(15)), "the oldest five were dropped");
        hs.intake(T0 + 100 + UNMATCHED_TTL_SECS, psbt(200), vec![], vec![]).unwrap();
        assert_eq!(hs.unmatched.len(), 1, "everything older than the TTL is gone");
    }

    #[test]
    fn a_read_closes_what_the_board_no_longer_lists_and_drops_its_bytes() {
        let mut hs = state();
        hs.reconcile(BOARD, &[0, 1], vec![sub(1, 900_000), sub(2, 950_000)], &[0, 1], T0);
        hs.intake(T0, psbt(1), vec![], vec![]).unwrap();
        // the hashes on chain are [1;32] and [2;32]; the bytes' sha256 differ, so match them by hand
        hs.seen[0].hash = offer_hash(&psbt(1));
        hs.intake(T0, psbt(1), vec![], vec![]).unwrap();
        assert_eq!(hs.bytes.len(), 1);
        // id 0 is claimed on chain: the next read lists only 1
        hs.reconcile(BOARD, &[1], vec![], &[], T0 + 10);
        assert!(!hs.seen(&BOARD, 0).unwrap().open);
        assert!(hs.seen(&BOARD, 1).unwrap().open);
        assert!(hs.bytes.is_empty(), "closed: its bytes are dropped");
        assert!(!hs.any_candidate());
    }

    #[test]
    fn bytes_that_never_arrive_are_invalid_after_the_slash_delay() {
        let mut hs = state();
        hs.reconcile(BOARD, &[0], vec![sub(1, 900_000)], &[0], T0);
        hs.reconcile(BOARD, &[0], vec![], &[], T0 + 3_599);
        assert!(hs.verdict_of(&BOARD, 0).is_none());
        hs.reconcile(BOARD, &[0], vec![], &[], T0 + 3_600);
        assert!(matches!(hs.verdict_of(&BOARD, 0), Some(Verdict::Invalid { why }) if why.contains("never arrived")));
        assert_eq!(hs.slashes_due(), vec![(BOARD, 0)]);
        // closed on chain (the submitter refunded at expiry before we slashed): no longer due
        hs.reconcile(BOARD, &[], vec![], &[], T0 + 90_000);
        assert!(hs.slashes_due().is_empty());
    }

    #[test]
    fn the_cheapest_open_hand_in_under_the_cap_with_bytes_and_time_left_is_chosen() {
        let mut hs = state();
        let subs = vec![sub(1, 950_000), sub(2, 900_000), sub(3, 800_000), sub(4, 900_000)];
        hs.reconcile(BOARD, &[0, 1, 2, 3], subs, &[0, 1, 2, 3], T0);
        for (i, tag) in [(0usize, 1u8), (1, 2), (2, 3), (3, 4)] {
            hs.seen[i].hash = offer_hash(&psbt(tag));
        }
        let clock = Clock { time: T0, parent_block: B0 };
        assert!(hs.cheapest(1_000_000, &clock, 4 * 3_600).is_none(), "no bytes yet");
        for tag in [1, 2, 4] {
            hs.intake(T0, psbt(tag), vec![0], vec![]).unwrap();
        }
        // 3 (the cheapest) has no bytes; of 2 and 4 at the same ask, the older wins
        let c = hs.cheapest(1_000_000, &clock, 4 * 3_600).unwrap();
        assert_eq!((c.id, c.ask_sats), (1, 900_000));
        assert_eq!(c.psbt, psbt(2));
        // under a lower cap, nothing
        assert!(hs.cheapest(899_999, &clock, 4 * 3_600).is_none());
        // a verdict takes it out of the running; the next cheapest is chosen
        hs.set_verdict(BOARD, 1, Verdict::Gone { why: "sold".into() }, T0);
        assert_eq!(hs.cheapest(1_000_000, &clock, 4 * 3_600).unwrap().id, 3);
        // too near its expiry on the parent chain's clock: not started
        let late = Clock { time: T0, parent_block: B0 + 86_400 / 12 - 100 };
        assert!(hs.cheapest(1_000_000, &late, 4 * 3_600).is_none());
    }

    #[test]
    fn claims_follow_the_frog_and_failed_purchases_pay_nothing() {
        use crate::custody::Outpoint;
        use crate::frogbuy::Bought;
        let mut hs = state();
        hs.reconcile(BOARD, &[0, 1], vec![sub(1, 900_000), sub(2, 900_000)], &[0, 1], T0);
        hs.set_verdict(BOARD, 0, Verdict::Bought { frog: 7 }, T0);
        hs.set_verdict(BOARD, 1, Verdict::Bought { frog: 8 }, T0);
        let buy = Bought { inscription: "x".into(), outpoint: Outpoint { txid: [1; 32], vout: 1 }, seller: Outpoint { txid: [2; 32], vout: 0 },
            spend_sats: 1, fee_sats: 1, rate: 1, cost_wei: 1, raw: vec![], setup_txid: None };
        let mut frogs = vec![
            FrogRec { id: 7, target: None, phase: FPhase::Sent { buy: buy.clone() } },
            FrogRec { id: 8, target: None, phase: FPhase::Signed { buy: buy.clone() } },
        ];
        assert!(hs.claims_due(&frogs).is_empty(), "not held yet");
        frogs[0].phase = FPhase::Held { buy: buy.clone() };
        assert_eq!(hs.claims_due(&frogs), vec![(BOARD, 0, 7)]);
        frogs[1].phase = FPhase::Failed { why: "the listing vanished".into() };
        hs.settle_failed(&frogs, T0 + 1);
        assert!(matches!(hs.verdict_of(&BOARD, 1), Some(Verdict::Gone { .. })));
        assert_eq!(hs.claims_due(&frogs).len(), 1);
        // claimed on chain (the read no longer lists it): not due again
        hs.set_verdict(BOARD, 0, Verdict::Claimed, T0 + 2);
        hs.reconcile(BOARD, &[1], vec![], &[], T0 + 2);
        assert!(hs.claims_due(&frogs).is_empty());
        // a stranded purchase pays nothing either
        hs.reconcile(BOARD, &[1, 2], vec![sub(3, 900_000)], &[2], T0 + 3);
        hs.set_verdict(BOARD, 2, Verdict::Bought { frog: 9 }, T0 + 3);
        frogs.push(FrogRec { id: 9, target: None, phase: FPhase::Stranded { buy, why: "runes".into() } });
        hs.settle_failed(&frogs, T0 + 4);
        assert!(matches!(hs.verdict_of(&BOARD, 2), Some(Verdict::Gone { why }) if why.contains("stranded")));
    }

    /// Audit D, L-3: every caller idle for an hour is forgotten, not only the one calling, and the table is bounded.
    #[test]
    fn the_rate_table_forgets_every_idle_caller_and_stays_bounded() {
        let mut r = Rate::default();
        for i in 0..500 {
            assert!(r.allow(&format!("fresh-{i}"), 1_000));
        }
        assert_eq!(r.len(), 500);
        assert!(r.allow("later", 1_000 + 3_600));
        assert_eq!(r.len(), 1, "the 500 idle callers are gone");
        for i in 0..(MAX_RATE_ENTRIES + 50) {
            r.allow(&format!("burst-{i}"), 10_000);
        }
        assert_eq!(r.len(), MAX_RATE_ENTRIES);
    }

    #[test]
    fn the_per_caller_rate_is_a_sliding_hour() {
        let mut r = Rate::default();
        for i in 0..SUBMITS_PER_HOUR {
            assert!(r.allow("alice", T0 + i as u64), "{i}");
        }
        assert!(!r.allow("alice", T0 + 100));
        assert!(r.allow("bob", T0 + 100), "another caller");
        assert!(r.allow("alice", T0 + 3_600), "the first slot is an hour old");
        assert!(!r.allow("alice", T0 + 3_600));
    }

    #[test]
    fn the_page_counts_open_hand_ins_and_bytes() {
        let mut hs = state();
        hs.reconcile(BOARD, &[0, 1], vec![sub(1, 1), sub(2, 2)], &[0, 1], T0);
        hs.seen[0].hash = offer_hash(&psbt(1));
        hs.intake(T0, psbt(1), vec![], vec![]).unwrap();
        hs.intake(T0, psbt(9), vec![], vec![]).unwrap();
        let v = views(&hs);
        assert_eq!((v[0].open, v[0].with_bytes, v[0].unmatched), (2, 1, 1));
        assert_eq!(v[0].params, Some(params()));
    }
}
