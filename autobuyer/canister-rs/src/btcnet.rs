//! The Bitcoin broadcast and read-back over HTTPS outcalls (`docs/v2/btc-broadcast.md`). The pure half is [`crate::btcsend`].
//!
//! One node sends (`is_replicated: false`) to each configured explorer, and its replies are only reported. What the buyer
//! acts on is [`btc_tx_seen`]: every node asks every explorer, and they must all agree, on a txid the canister computed from
//! the bytes it holds. Before anything is sent or believed, every explorer must show the installed network's checkpoint
//! block, so a provider on another chain is refused rather than averaged in.

use candid::CandidType;
use ic_cdk::management_canister::{
    http_request, HttpHeader, HttpMethod, HttpRequestArgs, HttpRequestResult, TransformArgs, TransformContext, TransformFunc,
};
use serde::Deserialize;

use crate::btcsend::{checkpoint, classify, display, from_display, parse_signed, status_view, BtcSendOutcome};
use crate::canister::{cfg, note, only_operator};

/// What the explorers, together, say about a transaction.
#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum BtcSeen {
    /// No explorer has it.
    NotYet,
    /// Every explorer has it, unconfirmed.
    Mempool,
    /// In a block, the same block on every explorer.
    Confirmed { height: u64, block_hash: String },
    /// The explorers did not agree, or a read failed. Nothing is believed; ask again later.
    Unsure(String),
}

/// What one broadcast did: each explorer's report, and what they then said together.
#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct BtcSend {
    /// Computed here from the bytes: the only txid ever watched.
    pub txid: String,
    /// Each explorer's reply to the single-node send. Reports only.
    pub providers: Vec<(String, BtcSendOutcome)>,
    /// The replicated read-back straight after. `NotYet` or `Mempool` is normal; ask [`btc_tx_status`] later.
    pub seen: BtcSeen,
}

fn transform(method: &str) -> Option<TransformContext> {
    Some(TransformContext {
        function: TransformFunc(candid::Func { principal: ic_cdk::api::canister_self(), method: method.into() }),
        context: vec![],
    })
}

async fn outcall(url: String, method: HttpMethod, body: Option<Vec<u8>>, replicated: bool, max: u64, t: &str)
    -> Result<(u16, Vec<u8>), String> {
    let req = HttpRequestArgs {
        url,
        method,
        body,
        max_response_bytes: Some(max),
        transform: transform(t),
        headers: vec![HttpHeader { name: "content-type".into(), value: "text/plain".into() }],
        is_replicated: Some(replicated),
    };
    let res: HttpRequestResult = http_request(&req).await.map_err(|e| format!("{e:?}"))?;
    Ok((res.status.to_string().parse().unwrap_or(0), res.body))
}

/// A transaction's presence, reduced before consensus to `present`/`absent`, so that a body which differs only in
/// presentation (fee estimates, a status that moved between two nodes' requests) cannot break agreement on the one fact read.
#[ic_cdk::query]
fn btc_presence(args: TransformArgs) -> HttpRequestResult {
    let status: u16 = args.response.status.to_string().parse().unwrap_or(0);
    let body = match status {
        200 => b"present".to_vec(),
        404 => b"absent".to_vec(),
        _ => format!("error {status}").into_bytes(),
    };
    HttpRequestResult { status: 200u32.into(), body, headers: vec![] }
}

/// The installed network's checkpoint, on every explorer. Refuses a network with no checkpoint (mainnet has none).
async fn assert_btc_network() -> Result<(), String> {
    let c = cfg();
    let (height, want) = checkpoint(&c.btc_network)
        .ok_or_else(|| format!("refused: no checkpoint for {:?}, so nothing is sent or believed there", c.btc_network))?;
    for base in &c.btc_api_urls {
        let (st, body) = outcall(format!("{base}/block-height/{height}"), HttpMethod::GET, None, true, 2_000, "strip_volatile").await?;
        let got = String::from_utf8_lossy(&body).trim().to_ascii_lowercase();
        if st != 200 || got != want {
            return Err(format!(
                "refused: {base} is not {:?}: block {height} there is {got:?} (HTTP {st}), not {want}", c.btc_network));
        }
    }
    Ok(())
}

thread_local! {
    /// The explorer list the checkpoint was last shown on, and when (heap: an upgrade, which may change the list, checks again).
    static CHECKED: std::cell::RefCell<Option<(Vec<String>, u64)>> = const { std::cell::RefCell::new(None) };
}

/// How long a checkpoint check stands for reads (audit D, I-3).
const CHECK_STANDS_SECS: u64 = 24 * 3600;

/// The checkpoint, for READS (audit D, I-3): before custody or the frog side believes anything an explorer says, every explorer
/// must have shown the installed network's checkpoint, for this explorer list, within the last day. A broadcast still checks
/// every time ([`broadcast_btc`]).
pub(crate) async fn ensure_btc_network() -> Result<(), String> {
    let urls = cfg().btc_api_urls;
    let now = ic_cdk::api::time() / 1_000_000_000;
    let fresh = CHECKED.with(|c| c.borrow().as_ref().is_some_and(|(u, at)| *u == urls && now.saturating_sub(*at) < CHECK_STANDS_SECS));
    if fresh {
        return Ok(());
    }
    assert_btc_network().await?;
    CHECKED.with(|c| *c.borrow_mut() = Some((urls, now)));
    Ok(())
}

/// Send signed bytes to every configured explorer from ONE node each, then read the chain back, replicated. Safe to repeat
/// with the same bytes: they spend the same inputs, so at most one of them is ever confirmed.
pub async fn broadcast_btc(raw_tx: &[u8]) -> Result<BtcSend, String> {
    let tx = parse_signed(raw_tx).map_err(|e| format!("refused: not one signed transaction: {e}"))?;
    let txid = tx.txid_hex();
    assert_btc_network().await?;
    let body = hex::encode(raw_tx).into_bytes();
    let mut providers = Vec::new();
    for base in &cfg().btc_api_urls {
        let outcome = match outcall(format!("{base}/tx"), HttpMethod::POST, Some(body.clone()), false, 4_000, "strip_volatile").await {
            Ok((st, reply)) => classify(st, &reply, &txid),
            Err(e) => BtcSendOutcome::Unreadable(format!("the outcall itself failed: {e}")),
        };
        providers.push((base.clone(), outcome));
    }
    let seen = btc_tx_seen(tx.txid).await;
    note(format!("btc broadcast {txid}: {:?}", seen));
    Ok(BtcSend { txid, providers, seen })
}

/// The read-back: what every explorer, asked by every node, says about `txid` (internal byte order). This, not the send, is
/// what the buyer acts on.
pub async fn btc_tx_seen(txid: [u8; 32]) -> BtcSeen {
    let t = display(&txid);
    let urls = cfg().btc_api_urls;
    let mut views = Vec::new();
    for base in &urls {
        match outcall(format!("{base}/tx/{t}/status"), HttpMethod::GET, None, true, 4_000, "strip_volatile").await {
            Ok((200, body)) => match status_view(&body) {
                Ok(v) => views.push(v),
                Err(e) => return BtcSeen::Unsure(format!("{base}: {e}")),
            },
            Ok((st, _)) => return BtcSeen::Unsure(format!("{base}: HTTP {st} on the status")),
            Err(e) => return BtcSeen::Unsure(format!("{base}: {e}")),
        }
    }
    let Some(first) = views.first().cloned() else { return BtcSeen::Unsure("no Bitcoin sources".into()) };
    if views.iter().any(|v| *v != first) {
        return BtcSeen::Unsure(format!("explorers disagree on {t}; nothing is believed"));
    }
    if let (true, Some(height), Some(block_hash)) = first {
        return BtcSeen::Confirmed { height, block_hash };
    }
    // Unconfirmed: the status answers {"confirmed":false} even for a txid nobody has heard of, so ask whether it exists.
    let mut present = Vec::new();
    for base in &urls {
        match outcall(format!("{base}/tx/{t}"), HttpMethod::GET, None, true, 64_000, "btc_presence").await {
            Ok((_, b)) if b == b"present" => present.push(true),
            Ok((_, b)) if b == b"absent" => present.push(false),
            Ok((_, b)) => return BtcSeen::Unsure(format!("{base}: {}", String::from_utf8_lossy(&b))),
            Err(e) => return BtcSeen::Unsure(format!("{base}: {e}")),
        }
    }
    match (present.iter().all(|p| *p), present.iter().all(|p| !*p)) {
        (true, _) => BtcSeen::Mempool,
        (_, true) => BtcSeen::NotYet,
        _ => BtcSeen::Unsure(format!("explorers disagree on whether they have {t}")),
    }
}

/// The rehearsal's broadcast: the same [`broadcast_btc`] the loop uses, behind `rehearse_sign_sighash`'s guards: installed
/// for the rehearsal network, and the caller names it again.
#[ic_cdk::update]
pub async fn rehearse_broadcast_btc(network: String, raw_hex: String) -> Result<BtcSend, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::canister::check_btc_rehearsal(&cfg().btc_network, &network, "sends")?;
    let raw = hex::decode(raw_hex.trim()).map_err(|e| format!("the transaction is not hex: {e}"))?;
    broadcast_btc(&raw).await
}

/// Ask the explorers again about a transaction, replicated. Operator only: each call spends cycles.
#[ic_cdk::update]
pub async fn btc_tx_status(txid_hex: String) -> BtcSeen {
    only_operator();
    let txid = match from_display(&txid_hex) {
        Ok(t) => t,
        Err(e) => return BtcSeen::Unsure(e),
    };
    if let Err(e) = assert_btc_network().await {
        return BtcSeen::Unsure(e);
    }
    btc_tx_seen(txid).await
}

#[cfg(test)]
mod tests {
    /// The same check canister.rs makes over its own file: every update method here starts with a caller guard.
    #[test]
    fn every_update_method_here_is_guarded() {
        let src = include_str!("btcnet.rs");
        let parts: Vec<&str> = src.split(concat!("#[ic_cdk::update]", "\npub async fn")).skip(1).collect();
        assert_eq!(parts.len(), 2, "rehearse_broadcast_btc and btc_tx_status");
        for p in parts {
            let head: String = p.chars().take(300).collect();
            assert!(head.contains("only_operator();"), "unguarded: {head}");
        }
    }
}
