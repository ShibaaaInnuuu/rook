//! The memo ledger (canister-only design §4.4 rule 4): every Rook transaction that changes a supply or a backing balance carries
//! `rook:v1:<coll>:<kind>:<entry id>[:i<intent>]` as an SPL Memo, so the books can be rebuilt from Solana history. The entry id is the
//! link's id (`chainlink::Ledger`), the optional suffix the desk intent it serves. `<coll>` is `-` for a transaction that touches no
//! collection (a refund of an intent's deposit).
//!
//! A memo is a LABEL (unauthenticated, anyone can write one in their own transaction): it decides which link a landed transaction is
//! read as, never whether money moved (that is the balance / supply rule, `chainlink::recon`).
use super::ledger::OpKind;

pub const PREFIX: &str = "rook:v1:";

/// A parsed memo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoRef {
    pub coll: Option<u16>,
    pub kind: String,
    pub id: u64,
    pub intent: Option<u64>,
}

/// The memo text of link `id`.
pub fn memo(coll: Option<u16>, kind: OpKind, id: u64, intent: Option<u64>) -> String {
    let c = coll.map(|c| c.to_string()).unwrap_or_else(|| "-".into());
    let i = intent.map(|n| format!(":i{n}")).unwrap_or_default();
    format!("{PREFIX}{c}:{}:{id}{i}", kind.tag())
}

/// A parsed memo as text again.
pub fn memo_text(m: &MemoRef) -> String {
    let c = m.coll.map(|c| c.to_string()).unwrap_or_else(|| "-".into());
    let i = m.intent.map(|n| format!(":i{n}")).unwrap_or_default();
    format!("{PREFIX}{c}:{}:{}{i}", m.kind, m.id)
}

/// One memo as written (no RPC decoration): `None` if it is not a Rook v1 memo.
pub fn parse(text: &str) -> Option<MemoRef> {
    let rest = text.trim().strip_prefix(PREFIX)?;
    let mut it = rest.split(':');
    let coll = match it.next()? {
        "-" => None,
        c => Some(c.parse::<u16>().ok()?),
    };
    let kind = it.next()?.to_string();
    if kind.is_empty() || !kind.chars().all(|c| c.is_ascii_lowercase()) {
        return None;
    }
    let id = it.next()?.parse::<u64>().ok()?;
    let intent = match it.next() {
        None => None,
        Some(s) => Some(s.strip_prefix('i')?.parse::<u64>().ok()?),
    };
    if it.next().is_some() {
        return None;
    }
    Some(MemoRef { coll, kind, id, intent })
}

/// The Rook memos in what `getSignaturesForAddress` reports as a signature's memo: `[<len>] <text>`, several joined by `; `.
pub fn parse_all(rpc_memo: &str) -> Vec<MemoRef> {
    rpc_memo
        .split("; ")
        .map(|part| {
            let p = part.trim();
            match p.strip_prefix('[').and_then(|r| r.split_once("] ")) {
                Some((n, text)) if n.chars().all(|c| c.is_ascii_digit()) => text,
                _ => p,
            }
        })
        .filter_map(parse)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_memo_round_trips_and_the_rpc_decoration_is_stripped() {
        let m = memo(Some(3), OpKind::Mint, 42, Some(7));
        assert_eq!(m, "rook:v1:3:mint:42:i7");
        assert_eq!(parse(&m), Some(MemoRef { coll: Some(3), kind: "mint".into(), id: 42, intent: Some(7) }));
        let r = memo(None, OpKind::Refund, 9, None);
        assert_eq!(r, "rook:v1:-:refund:9");
        assert_eq!(parse(&r).unwrap().coll, None);
        assert_eq!(parse_all(&format!("[{}] {m}; [3] hi; [{}] {r}", m.len(), r.len())).len(), 2);
        for bad in ["rook:v2:3:mint:1", "rook:v1:x:mint:1", "rook:v1:3:Mint:1", "rook:v1:3:mint:1:7", "rook:v1:3:mint:1:i7:9", "hello"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }
}
