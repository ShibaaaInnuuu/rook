//! The reference rate and the runes indexer, as the canister reads them (`docs/v2/rates-and-indexer.md`). Every read is a
//! replicated outcall reduced by a transform to the one fact read; the decisions are `rates` and `runeindex`'s, which are pure.
//! Read-only: nothing here sends, signs or spends.

use std::cell::RefCell;
use std::collections::BTreeMap;

use candid::{CandidType, Deserialize, Principal};
use ic_cdk::management_canister::{
    http_request, HttpHeader, HttpMethod, HttpRequestArgs, HttpRequestResult, TransformArgs, TransformContext, TransformFunc,
};

use crate::canister::{cfg, note, only_operator};
use crate::psbt::Quote;
use crate::rates::{self, Market, Rate, SolBtcRate, SolRate, Source};
use crate::runeindex::{self, Evidence, Holding, OrdOutput, TxOutputs};
use crate::runestone::RuneId;

/// Bitcoin mainnet's keyless runes indexers and explorers. There are none for signet or testnet4 (see the design doc).
const MAINNET_ORD: &str = "https://ordinals.com";
const MAINNET_OW: &str = "https://turbo.ordinalswallet.com";
const MAINNET_EXPLORERS: [&str; 2] = ["https://mempool.space/api", "https://blockstream.info/api"];
/// The IC's exchange rate canister, on the IC mainnet only.
const XRC: &str = "uf6dk-hyaaa-aaaaq-qaaaq-cai";
const XRC_CYCLES: u128 = 1_000_000_000;

thread_local! {
    /// Names decoded from etchings. A rune's name never changes, so it is decoded once per install.
    static NAMES: RefCell<BTreeMap<(u64, u32), String>> = const { RefCell::new(BTreeMap::new()) };
}

// ---- the outcall and its transforms -------------------------------------------------------------------------------------

/// One replicated GET, reduced by `transform` before consensus. The transform always answers HTTP 200 with a canonical body
/// that starts `ok ` or `err `, so a failure agrees across replicas as surely as a success.
async fn get(url: String, transform: &str, context: Vec<u8>, max: u64) -> Result<String, String> {
    let req = HttpRequestArgs {
        url,
        method: HttpMethod::GET,
        body: None,
        max_response_bytes: Some(max),
        transform: Some(TransformContext {
            function: TransformFunc(candid::Func { principal: ic_cdk::api::canister_self(), method: transform.into() }),
            context,
        }),
        headers: vec![
            HttpHeader { name: "user-agent".into(), value: "etch-autobuyer".into() },
            HttpHeader { name: "accept".into(), value: "application/json".into() },
        ],
        is_replicated: Some(true),
    };
    let res = http_request(&req).await.map_err(|e| format!("{e:?}"))?;
    let body = String::from_utf8(res.body).map_err(|_| "a transformed body that is not text".to_string())?;
    match body.split_once(' ') {
        Some(("ok", rest)) => Ok(rest.to_string()),
        Some(("err", rest)) => Err(rest.to_string()),
        _ => Err(format!("an unexpected transformed body: {body:?}")),
    }
}

fn reply(r: Result<String, String>) -> HttpRequestResult {
    let body = match r {
        Ok(s) => format!("ok {s}"),
        Err(e) => format!("err {e}"),
    };
    HttpRequestResult { status: 200u32.into(), body: body.into_bytes(), headers: vec![] }
}

fn status_of(a: &TransformArgs) -> u16 {
    a.response.status.to_string().parse().unwrap_or(0)
}

/// A ticker, reduced to its quantised price. The context's first byte names the source, the second (when present) the market
/// (absent: ETH-BTC, as before the Solana home).
#[ic_cdk::query]
fn oracle_rate_view(args: TransformArgs) -> HttpRequestResult {
    reply(match (Source::from_context(&args.context), Market::from_context(&args.context)) {
        (Some(s), Some(m)) => s.project_market(m, status_of(&args), &args.response.body).map(|v| v.to_string()),
        _ => Err("unknown rate source or market".into()),
    })
}

/// ord's `/r/utxo`, reduced to its runes by name (sorted), or `notfound`.
#[ic_cdk::query]
fn oracle_ord_view(args: TransformArgs) -> HttpRequestResult {
    reply(runeindex::parse_ord_utxo(status_of(&args), &args.response.body).map(|o| match o {
        OrdOutput::NotFound => "notfound".to_string(),
        OrdOutput::Found(m) => {
            let m: BTreeMap<String, String> = m.into_iter().map(|(k, v)| (k, v.to_string())).collect();
            format!("found {}", serde_json::to_string(&m).unwrap_or_default())
        }
    }))
}

/// Ordinals Wallet's address balance, reduced to sorted (id, name, amount) triples; icons and the rest are dropped.
#[ic_cdk::query]
fn oracle_ow_view(args: TransformArgs) -> HttpRequestResult {
    reply(runeindex::parse_ow_balance(status_of(&args), &args.response.body).map(|mut hs| {
        hs.sort_by(|a, b| (a.id.block, a.id.tx).cmp(&(b.id.block, b.id.tx)));
        let rows: Vec<(String, String, String)> =
            hs.into_iter().map(|h| (format!("{}:{}", h.id.block, h.id.tx), h.name, h.amount.to_string())).collect();
        serde_json::to_string(&rows).unwrap_or_default()
    }))
}

/// An esplora `/tx/<txid>`, reduced to its confirmation height and its outputs' scripts and addresses.
#[ic_cdk::query]
fn oracle_tx_view(args: TransformArgs) -> HttpRequestResult {
    reply(runeindex::parse_esplora_tx(status_of(&args), &args.response.body).map(|t| {
        serde_json::to_string(&(t.height, t.outputs)).unwrap_or_default()
    }))
}

/// A short text answer (a block hash, a txid, a height), trimmed; anything else is an error.
#[ic_cdk::query]
fn oracle_text_view(args: TransformArgs) -> HttpRequestResult {
    let st = status_of(&args);
    let t = String::from_utf8_lossy(&args.response.body).trim().to_string();
    reply(if st == 200 && !t.is_empty() && t.len() <= 80 && t.chars().all(|c| c.is_ascii_alphanumeric()) {
        Ok(t)
    } else {
        Err(format!("HTTP {st}"))
    })
}

// ---- the reference rate ------------------------------------------------------------------------------------------------

#[derive(CandidType, Deserialize, Debug)]
enum AssetClass { Cryptocurrency, FiatCurrency }
#[derive(CandidType, Deserialize, Debug)]
struct Asset { symbol: String, class: AssetClass }
#[derive(CandidType, Deserialize, Debug)]
struct GetExchangeRateRequest { base_asset: Asset, quote_asset: Asset, timestamp: Option<u64> }
#[derive(CandidType, Deserialize, Debug)]
struct ExchangeRateMetadata {
    decimals: u32,
    base_asset_num_received_rates: u64,
    base_asset_num_queried_sources: u64,
    quote_asset_num_received_rates: u64,
    quote_asset_num_queried_sources: u64,
    standard_deviation: u64,
    forex_timestamp: Option<u64>,
}
#[derive(CandidType, Deserialize, Debug)]
struct ExchangeRate { base_asset: Asset, quote_asset: Asset, timestamp: u64, rate: u64, metadata: ExchangeRateMetadata }
#[derive(CandidType, Deserialize, Debug)]
enum ExchangeRateError {
    AnonymousPrincipalNotAllowed, Pending, CryptoBaseAssetNotFound, CryptoQuoteAssetNotFound, StablecoinRateNotFound,
    StablecoinRateTooFewRates, StablecoinRateZeroRate, ForexInvalidTimestamp, ForexBaseAssetNotFound, ForexQuoteAssetNotFound,
    ForexAssetsNotFound, RateLimited, NotEnoughCycles, FailedToAcceptCycles, InconsistentRatesReceived,
    Other { code: u32, description: String },
}
#[derive(CandidType, Deserialize, Debug)]
enum GetExchangeRateResult { Ok(ExchangeRate), Err(ExchangeRateError) }

/// ETH/BTC from the IC's exchange rate canister, or why not. Absent on a local replica, which is expected.
async fn xrc_reading() -> Result<u64, String> {
    let r = xrc_rate("ETH", "BTC").await?;
    let now = ic_cdk::api::time() / 1_000_000_000;
    rates::xrc_sats_per_eth(r.rate, r.metadata.decimals, r.timestamp, now, r.metadata.base_asset_num_received_rates,
        r.metadata.quote_asset_num_received_rates)
}

/// SOL/ETH (base SOL, quote ETH) from the exchange rate canister, as lamports per ETH.
async fn xrc_sol_reading() -> Result<u64, String> {
    let r = xrc_rate("SOL", "ETH").await?;
    let now = ic_cdk::api::time() / 1_000_000_000;
    rates::xrc_lamports_per_eth(r.rate, r.metadata.decimals, r.timestamp, now, r.metadata.base_asset_num_received_rates,
        r.metadata.quote_asset_num_received_rates)
}

/// SOL/BTC (base SOL, quote BTC) from the exchange rate canister, as satoshis per SOL.
async fn xrc_sol_btc_reading() -> Result<u64, String> {
    let r = xrc_rate("SOL", "BTC").await?;
    let now = ic_cdk::api::time() / 1_000_000_000;
    rates::xrc_sats_per_sol(r.rate, r.metadata.decimals, r.timestamp, now, r.metadata.base_asset_num_received_rates,
        r.metadata.quote_asset_num_received_rates)
}

/// One pair from the exchange rate canister.
async fn xrc_rate(base: &str, quote: &str) -> Result<ExchangeRate, String> {
    let xrc = Principal::from_text(XRC).map_err(|e| format!("xrc: {e}"))?;
    let crypto = |s: &str| Asset { symbol: s.into(), class: AssetClass::Cryptocurrency };
    let req = GetExchangeRateRequest { base_asset: crypto(base), quote_asset: crypto(quote), timestamp: None };
    let res = ic_cdk::call::Call::bounded_wait(xrc, "get_exchange_rate")
        .with_arg(req)
        .with_cycles(XRC_CYCLES)
        .await
        .map_err(|e| format!("xrc: {e:?}"))?;
    match res.candid::<GetExchangeRateResult>().map_err(|e| format!("xrc: {e:?}"))? {
        GetExchangeRateResult::Ok(r) => Ok(r),
        GetExchangeRateResult::Err(e) => Err(format!("xrc: {e:?}")),
    }
}

/// Every source read now, then the median, spread and count tests. There is no cached rate.
pub async fn read_rate() -> Result<Rate, String> {
    let mut readings = Vec::new();
    for s in Source::ALL {
        let r = get(s.url().into(), "oracle_rate_view", s.context(), rates::TICKER_BYTES).await
            .and_then(|b| b.parse::<u64>().map_err(|_| format!("{}: unreadable", s.name())));
        readings.push((s.name().to_string(), r));
    }
    readings.push(("xrc".to_string(), xrc_reading().await));
    rates::aggregate(readings)
}

/// SOL in ETH for the Solana home (docs/v9/solana-migration.md §2.3): every source's ETH-USD over its SOL-USD (each leg its own
/// replicated read, quantised before consensus), and the exchange rate canister's SOL/ETH; the same median, spread and count tests
/// as ETH/BTC (`rates::aggregate_sol`). There is no cached rate.
pub async fn read_sol_rate() -> Result<SolRate, String> {
    let mut readings = Vec::new();
    for s in Source::ALL {
        let leg = |m: Market| async move {
            get(s.url_of(m), "oracle_rate_view", s.context_of(m), rates::TICKER_BYTES).await
                .and_then(|b| b.parse::<u64>().map_err(|_| format!("{}: unreadable", s.name())))
        };
        let r = match (leg(Market::EthUsd).await, leg(Market::SolUsd).await) {
            (Ok(eth), Ok(sol)) => rates::lamports_per_eth(eth, sol).map_err(|e| format!("{}: {e}", s.name())),
            (Err(e), _) | (_, Err(e)) => Err(e),
        };
        readings.push((s.name().to_string(), r));
    }
    readings.push(("xrc".to_string(), xrc_sol_reading().await));
    let r = rates::aggregate_sol(readings)?;
    note(format!("SOL/ETH {} lamports per ETH (spread {} bps)", r.lamports_per_eth, r.spread_bps));
    Ok(r)
}

/// SOL in BTC for the frogs on the Solana home (phase B2; docs/v9/solana-migration.md §2.3): every source's SOL-USD over its BTC-USD
/// (each leg its own replicated read, quantised before consensus), and the exchange rate canister's SOL/BTC; the same median, spread
/// and count tests as ETH/BTC (`rates::aggregate_sol_btc`). There is no cached rate.
pub async fn read_sol_btc_rate() -> Result<SolBtcRate, String> {
    let mut readings = Vec::new();
    for s in Source::ALL {
        let leg = |m: Market| async move {
            get(s.url_of(m), "oracle_rate_view", s.context_of(m), rates::TICKER_BYTES).await
                .and_then(|b| b.parse::<u64>().map_err(|_| format!("{}: unreadable", s.name())))
        };
        let r = match (leg(Market::BtcUsd).await, leg(Market::SolUsd).await) {
            (Ok(btc), Ok(sol)) => rates::sats_per_sol(btc, sol).map_err(|e| format!("{}: {e}", s.name())),
            (Err(e), _) | (_, Err(e)) => Err(e),
        };
        readings.push((s.name().to_string(), r));
    }
    readings.push(("xrc".to_string(), xrc_sol_btc_reading().await));
    let r = rates::aggregate_sol_btc(readings)?;
    note(format!("SOL/BTC {} sats per SOL (spread {} bps)", r.sats_per_sol, r.spread_bps));
    Ok(r)
}

/// The loop's hook: the least BTC `send_wei` is worth by our reference rate, less the tolerance.
pub async fn reference_floor(send_wei: u128) -> Result<u64, String> {
    let rate = read_rate().await?;
    let floor = rates::floor_sats(send_wei, rate.sats_per_eth)?;
    note(format!("reference rate {} sat/ETH (spread {} bps); floor {floor} sat for {send_wei} wei", rate.sats_per_eth, rate.spread_bps));
    Ok(floor)
}

/// The quote check's hook (`docs/v2/trust-points.md` §1): the most `wei` is worth by our reference rate, plus the tolerance. The
/// same live read as `reference_floor`: one rate path for the bridge judge and the quote cap.
pub async fn reference_cap(wei: u128) -> Result<u64, String> {
    let rate = read_rate().await?;
    let cap = rates::cap_sats(wei, rate.sats_per_eth)?;
    note(format!("reference rate {} sat/ETH (spread {} bps); cap {cap} sat for {wei} wei", rate.sats_per_eth, rate.spread_bps));
    Ok(cap)
}

/// The rate and every source's reading, now. Operator-only: each call spends cycles on six outcalls and one XRC call.
#[ic_cdk::update]
pub async fn reference_rate() -> Result<Rate, String> {
    only_operator();
    read_rate().await
}

// ---- the runes indexer ---------------------------------------------------------------------------------------------------

struct Indexers {
    explorers: Vec<String>,
    ord: String,
    ow: String,
}

fn mainnet_indexers() -> Indexers {
    Indexers {
        explorers: MAINNET_EXPLORERS.iter().map(|s| s.to_string()).collect(),
        ord: MAINNET_ORD.into(),
        ow: MAINNET_OW.into(),
    }
}

/// The same fact from every explorer, and they must agree.
async fn agreed(explorers: &[String], path: &str, transform: &str, max: u64) -> Result<String, String> {
    let mut first: Option<String> = None;
    for base in explorers {
        let v = get(format!("{base}{path}"), transform, vec![], max).await.map_err(|e| format!("{base}: {e}"))?;
        match &first {
            None => first = Some(v),
            Some(f) if *f != v => return Err(format!("the explorers disagree on {path}; nothing is believed")),
            Some(_) => {}
        }
    }
    first.ok_or_else(|| "no explorers".to_string())
}

async fn tx_outputs(explorers: &[String], txid_hex: &str) -> Result<TxOutputs, String> {
    let v = agreed(explorers, &format!("/tx/{txid_hex}"), "oracle_tx_view", 200_000).await?;
    let (height, outputs): (Option<u64>, Vec<(u32, String, Option<String>)>) =
        serde_json::from_str(&v).map_err(|_| "an unreadable transaction view".to_string())?;
    Ok(TxOutputs { height, outputs })
}

/// The rune's spaced name, decoded from its etching transaction as the explorers serve it — never from an indexer.
async fn name_of(explorers: &[String], id: RuneId) -> Result<String, String> {
    if let Some(n) = NAMES.with(|m| m.borrow().get(&(id.block, id.tx)).cloned()) {
        return Ok(n);
    }
    let name = if id.block == 1 && id.tx == 0 {
        runeindex::etched_name(id, &[])?
    } else {
        let hash = agreed(explorers, &format!("/block-height/{}", id.block), "oracle_text_view", 1_000).await?;
        let txid = agreed(explorers, &format!("/block/{hash}/txid/{}", id.tx), "oracle_text_view", 1_000).await?;
        let tx = tx_outputs(explorers, &txid).await?;
        let scripts = tx.outputs.iter().map(|(_, s, _)| hex::decode(s)).collect::<Result<Vec<_>, _>>()
            .map_err(|_| "an etching script that is not hex".to_string())?;
        runeindex::etched_name(id, &scripts)?
    };
    NAMES.with(|m| m.borrow_mut().insert((id.block, id.tx), name.clone()));
    Ok(name)
}

async fn delivered_with(ix: &Indexers, txid_hex: &str, rune: RuneId, recipient: &[u8]) -> Result<u128, String> {
    let tx = tx_outputs(&ix.explorers, txid_hex).await?;
    let name = name_of(&ix.explorers, rune).await?;
    let outs = runeindex::delivery_outputs(&tx, recipient);
    let ord_height: u64 = get(format!("{}/r/blockheight", ix.ord), "oracle_text_view", vec![], 1_000).await
        .map_err(|e| format!("ord: {e}"))?
        .parse().map_err(|_| "ord: an unreadable height".to_string())?;
    let mut ord = Vec::new();
    for (vout, _) in &outs {
        let v = get(format!("{}/r/utxo/{txid_hex}:{vout}", ix.ord), "oracle_ord_view", vec![], 64_000).await
            .map_err(|e| format!("ord: {e}"))?;
        let o = if v == "notfound" {
            OrdOutput::NotFound
        } else {
            let m: BTreeMap<String, String> = serde_json::from_str(v.strip_prefix("found ").ok_or("ord: an unexpected view")?)
                .map_err(|_| "ord: an unreadable view".to_string())?;
            OrdOutput::Found(m.into_iter().map(|(k, a)| a.parse::<u128>().map(|n| (k, n)))
                .collect::<Result<_, _>>().map_err(|_| "ord: an unreadable amount".to_string())?)
        };
        ord.push((*vout, o));
    }
    let holdings: Vec<Holding> = match outs.first().and_then(|(_, a)| a.clone()) {
        None => Vec::new(),
        Some(addr) => {
            let v = get(format!("{}/wallet/{addr}/rune-balance", ix.ow), "oracle_ow_view", vec![], 100_000).await
                .map_err(|e| format!("ordinals wallet: {e}"))?;
            let rows: Vec<(String, String, String)> = serde_json::from_str(&v).map_err(|_| "ordinals wallet: an unreadable view")?;
            rows.into_iter().map(|(id, name, amount)| Ok(Holding {
                id: runeindex::parse_rune_id(&id).ok_or("ordinals wallet: an unreadable id")?,
                name,
                amount: amount.parse().map_err(|_| "ordinals wallet: an unreadable amount")?,
            })).collect::<Result<_, String>>()?
        }
    };
    runeindex::delivered(&Evidence { rune, name: &name, recipient_script: recipient, tx: &tx, ord_height, ord: &ord, holdings: &holdings })
}

/// The loop's hook: units of the quote's rune that transaction `txid` (display order) delivered to the quote's recipient.
/// Only Bitcoin mainnet has keyless runes indexers; on a test network this refuses, as before.
pub async fn delivered(txid: [u8; 32], quote: &Quote) -> Result<u128, String> {
    let network = cfg().btc_network;
    if network != crate::canister::BITCOIN_MAINNET {
        return Err(format!("no keyless runes indexer exists for {network}: a delivery cannot be read, so nothing settles on its own"));
    }
    let units = delivered_with(&mainnet_indexers(), &hex::encode(txid), quote.rune, &quote.recipient_script).await?;
    note(format!("delivery {}: {units} units of {}:{}", hex::encode(txid), quote.rune.block, quote.rune.tx));
    Ok(units)
}

/// Read-only diagnostics against Bitcoin MAINNET's indexers and explorers, whatever network this canister was installed for:
/// how many units of `rune_id` transaction `txid` delivered to `recipient_script_hex`. Operator-only: each call spends cycles.
#[ic_cdk::update]
pub async fn probe_delivered(txid: String, rune_id: String, recipient_script_hex: String) -> Result<String, String> {
    only_operator();
    let rune = runeindex::parse_rune_id(&rune_id).ok_or("rune_id is block:tx")?;
    let script = hex::decode(recipient_script_hex.trim()).map_err(|_| "recipient_script_hex is not hex")?;
    let t = txid.trim().to_ascii_lowercase();
    if t.len() != 64 || hex::decode(&t).is_err() {
        return Err("txid is 64 hex digits".into());
    }
    let ix = mainnet_indexers();
    let name = name_of(&ix.explorers, rune).await?;
    let units = delivered_with(&ix, &t, rune, &script).await?;
    Ok(format!("{units} {name}"))
}

/// Read-only: the spaced name of `rune_id`, decoded from its etching as Bitcoin mainnet's explorers serve it.
#[ic_cdk::update]
pub async fn probe_rune_name(rune_id: String) -> Result<String, String> {
    only_operator();
    let rune = runeindex::parse_rune_id(&rune_id).ok_or("rune_id is block:tx")?;
    name_of(&mainnet_indexers().explorers, rune).await
}

#[cfg(test)]
mod tests {
    /// Every update method here is guarded, as everywhere else in the canister.
    #[test]
    fn every_update_method_is_operator_only() {
        let src = include_str!("oracle.rs");
        let parts: Vec<&str> = src.split(concat!("#[ic_cdk::update]", "\npub async fn")).skip(1).collect();
        assert_eq!(parts.len(), 3);
        for p in parts {
            let head: String = p.chars().take(400).collect();
            assert!(head.contains("only_operator();"), "unguarded: {head}");
        }
    }
}
