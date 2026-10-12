//! The ordinals providers as the canister reads them (docs/v6/ordinals-apis.md §3): every read a replicated outcall reduced
//! by a transform to the one ord-shaped fact, every provider asked, and `ordapi::verified` deciding whether the answers are
//! one answer. Nothing is decided here beyond that; `custodynet.rs` hands what verifies to the flows exactly as ord's word
//! was handed before, and an unverified read is the missing answer those flows already wait on.
//!
//! Which providers: Bitcoin mainnet's keyless pair (ord's recursive endpoints at ordinals.com, Ordinals Wallet) and UniSat
//! once the operator has set its key. The key is the one secret this module holds: set by the operator, kept on the heap
//! (an upgrade clears it; set it again), never returned by a query, never written to the log, and sent in exactly one header
//! to exactly one host. Every replica of the subnet sees the outcall, so it is a read-only key on a plan whose loss costs a
//! quota, not money. An operator may also add an ord of their own (`add_ord_provider`): a self-hosted ord is one more
//! `Kind::Ord`, and two of them are two providers on a network no hosted API serves.

use std::cell::RefCell;

use ic_cdk::management_canister::{http_request, HttpHeader, HttpMethod, HttpRequestArgs, HttpRequestResult, TransformArgs,
    TransformContext, TransformFunc};

use crate::canister::{cfg, note, only_operator};
use crate::custody::{self, Contents, Location, Outpoint};
use crate::ordapi::{self, Answer, Kind, Provider};

thread_local! {
    /// UniSat's bearer key. Heap only, by design: nothing in stable memory, nothing in an upgrade argument (which is published).
    static UNISAT_KEY: RefCell<Option<String>> = const { RefCell::new(None) };
    /// ords the operator added, by base URL. Heap only, like the key.
    static EXTRA_ORDS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

const USER_AGENT: &str = "etch-autobuyer/1 (read-only)";
const MAX_EXTRA_ORDS: usize = 2;

/// The providers for the installed network: mainnet's, plus any ord the operator added; on another network only those.
pub(crate) fn providers() -> Vec<Provider> {
    let mut ps = if cfg().btc_network == crate::canister::BITCOIN_MAINNET { ordapi::mainnet_providers(UNISAT_KEY.with(|k| k.borrow().is_some())) } else { Vec::new() };
    EXTRA_ORDS.with(|e| ps.extend(e.borrow().iter().map(|b| Provider::ord(b))));
    ps
}

// ---- outcalls -----------------------------------------------------------------------------------------------------------------

/// One replicated GET to `p`, reduced by `transform` to `ok …` / `err …`. UniSat's key goes into this request's one header and nowhere else.
async fn get(p: &Provider, url: String, transform: &str, mut context: Vec<u8>, max: u64) -> Result<String, String> {
    let mut headers = vec![
        HttpHeader { name: "user-agent".into(), value: USER_AGENT.into() },
        HttpHeader { name: "accept".into(), value: "application/json".into() },
    ];
    if p.kind == Kind::UniSat {
        let key = UNISAT_KEY.with(|k| k.borrow().clone()).ok_or("unisat: no key is set")?;
        headers.push(HttpHeader { name: "authorization".into(), value: format!("Bearer {key}") });
    }
    context.insert(0, p.kind.byte());
    let req = HttpRequestArgs {
        url,
        method: HttpMethod::GET,
        body: None,
        max_response_bytes: Some(max),
        transform: Some(TransformContext {
            function: TransformFunc(candid::Func { principal: ic_cdk::api::canister_self(), method: transform.into() }),
            context,
        }),
        headers,
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

fn kind_of(a: &TransformArgs) -> Result<Kind, String> {
    a.context.first().copied().and_then(Kind::of_byte).ok_or_else(|| "no provider kind in the context".to_string())
}

// ---- transforms: one per fact, the provider's dialect in the context ---------------------------------------------------------------

/// The provider's height, as text.
#[ic_cdk::query]
fn ordapi_height(args: TransformArgs) -> HttpRequestResult {
    reply(kind_of(&args).and_then(|k| ordapi::parse_height(k, status_of(&args), &args.response.body)).map(|h| h.to_string()))
}

/// Where the provider says the inscription (the context after the kind byte) is: `none`, or `<outpoint> <offset> <value>`.
#[ic_cdk::query]
fn ordapi_location(args: TransformArgs) -> HttpRequestResult {
    let id = String::from_utf8(args.context.get(1..).unwrap_or_default().to_vec()).unwrap_or_default();
    reply(kind_of(&args).and_then(|k| ordapi::parse_location(k, status_of(&args), &args.response.body, &id)).map(|l| match l {
        None => "none".to_string(),
        Some(l) => format!("{} {} {}", l.output.text(), l.offset, l.value),
    }))
}

/// What the provider says an output carries (ord: the whole answer; UniSat: inscriptions and value, runes come separately): `none`, or JSON.
#[ic_cdk::query]
fn ordapi_contents(args: TransformArgs) -> HttpRequestResult {
    reply(kind_of(&args).and_then(|k| match k {
        Kind::Ord => ordapi::parse_contents(k, status_of(&args), &args.response.body, None),
        Kind::UniSat => ordapi::parse_unisat_utxo(status_of(&args), &args.response.body),
        Kind::OrdinalsWallet => Err("ordinalswallet: no per-output view".into()),
    }).map(|c| match c {
        None => "none".to_string(),
        Some(c) => serde_json::to_string(&c).unwrap_or_default(),
    }))
}

/// UniSat's runes on an output: a JSON list of names.
#[ic_cdk::query]
fn ordapi_runes(args: TransformArgs) -> HttpRequestResult {
    reply(kind_of(&args).and_then(|k| match k {
        Kind::UniSat => ordapi::parse_unisat_runes(status_of(&args), &args.response.body),
        _ => Err("only unisat answers runes separately".into()),
    }).map(|r| serde_json::to_string(&r).unwrap_or_default()))
}

// ---- the verified reads -----------------------------------------------------------------------------------------------------------

async fn height(p: &Provider) -> Result<u64, String> {
    get(p, p.url_height(), "ordapi_height", vec![], 4_000).await?.parse().map_err(|_| "an unreadable height".to_string())
}

fn location_of(v: &str) -> Result<Option<Location>, String> {
    if v == "none" {
        return Ok(None);
    }
    let p: Vec<&str> = v.split(' ').collect();
    if p.len() != 3 {
        return Err("an unreadable location".into());
    }
    Ok(Some(Location { output: Outpoint::parse(p[0])?, offset: p[1].parse().map_err(|_| "offset")?, value: p[2].parse().map_err(|_| "value")? }))
}

/// Every provider's account of `read`: its height before, the fact, its height after. A provider that fails anywhere is an
/// answer with an error in it, and `ordapi::verified` says so; the read goes on to the others so the reason names them all.
async fn ask<T, F>(ps: &[Provider], read: impl Fn(Provider) -> F) -> Vec<Answer<T>>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    let mut out = Vec::new();
    for p in ps {
        let height_before = height(p).await;
        let value = read(p.clone()).await;
        let height_after = height(p).await;
        out.push(Answer { provider: p.name.clone(), height_before, height_after, value });
    }
    out
}

/// Where `inscription` is, per every provider at one height, or `Err("unverified: …")`. `Ok(None)`: none knows it.
pub(crate) async fn locate_with(ps: &[Provider], inscription: &str) -> Result<Option<Location>, String> {
    custody::check_inscription_id(inscription)?;
    let answers = ask(ps, |p| async move {
        location_of(&get(&p, p.url_inscription(inscription), "ordapi_location", inscription.as_bytes().to_vec(), 16_000).await?)
    }).await;
    ordapi::verified(&format!("the location of {inscription}"), &answers).map(|(l, _)| l.clone())
}

/// What `o` carries, per every provider with a per-output view (ord, UniSat) at one height, or `Err("unverified: …")`.
pub(crate) async fn contents_with(ps: &[Provider], o: &Outpoint) -> Result<Option<Contents>, String> {
    let ps: Vec<Provider> = ps.iter().filter(|p| p.reads_contents()).cloned().collect();
    let answers = ask(&ps, |p| async move {
        let url = p.url_output(o).ok_or("no per-output view")?;
        let v = get(&p, url, "ordapi_contents", vec![], 64_000).await?;
        if v == "none" {
            return Ok(None);
        }
        let mut c: Contents = serde_json::from_str(&v).map_err(|_| "unreadable contents".to_string())?;
        if let Some(url) = p.url_runes(o) {
            c.runes = serde_json::from_str(&get(&p, url, "ordapi_runes", vec![], 64_000).await?).map_err(|_| "unreadable runes".to_string())?;
            c.runes = c.runes.iter().map(|r| ordapi::rune_name(r)).collect();
            c.runes.sort();
            c.runes.dedup();
        }
        Ok(Some(c))
    }).await;
    ordapi::verified(&format!("the contents of {}", o.text()), &answers).map(|(c, _)| c.clone())
}

/// Bitcoin mainnet's providers as of now (UniSat if its key is set), for a rehearsal on mainnet whatever the install is for.
pub(crate) fn mainnet_now() -> Vec<Provider> {
    ordapi::mainnet_providers(UNISAT_KEY.with(|k| k.borrow().is_some()))
}

pub(crate) async fn locate(inscription: &str) -> Result<Option<Location>, String> {
    locate_with(&providers(), inscription).await
}

pub(crate) async fn contents(o: &Outpoint) -> Result<Option<Contents>, String> {
    contents_with(&providers(), o).await
}

// ---- the operator's configuration ---------------------------------------------------------------------------------------------------

/// UniSat's key: read-only, sent in one header to open-api.unisat.io, visible to the subnet's replicas (an outcall is), lost at
/// an upgrade (set it again). Never echoed: the answer and the log say only that it was set.
#[ic_cdk::update]
pub fn set_unisat_key(key: String) -> Result<String, String> {
    only_operator();
    let key = key.trim().to_string();
    if key.is_empty() || key.len() > 256 || key.contains(char::is_whitespace) || !key.is_ascii() {
        return Err("refused: a key is one ASCII token of at most 256 characters".into());
    }
    let n = key.len();
    UNISAT_KEY.with(|k| *k.borrow_mut() = Some(key));
    note(format!("unisat key set ({n} characters); unisat is a provider from the next read"));
    Ok(format!("set ({n} characters, not shown)"))
}

/// Audit D, I-2 (accepted, documented): the operator can clear the key, and on mainnet UniSat is then no provider, so contents
/// cannot be verified by two providers and every buy and delivery waits (fail closed). That is a denial of service the
/// operator can already cause by not ticking; it can never make custody sign anything. An ord the operator adds can only
/// refuse: every provider must agree, and ordinals.com is always one of them on mainnet.
#[ic_cdk::update]
pub fn clear_unisat_key() -> Result<String, String> {
    only_operator();
    let had = UNISAT_KEY.with(|k| k.borrow_mut().take().is_some());
    note("unisat key cleared");
    Ok(if had { "cleared".into() } else { "there was none".into() })
}

/// An ord of one's own (its recursive endpoints), as one more provider. At most two, https only, never the same base twice.
#[ic_cdk::update]
pub fn add_ord_provider(base: String) -> Result<String, String> {
    only_operator();
    let base = ordapi::check_base(&base)?;
    if providers().iter().any(|p| p.base == base) {
        return Err(format!("refused: {base} is already a provider"));
    }
    EXTRA_ORDS.with(|e| {
        let mut e = e.borrow_mut();
        if e.len() >= MAX_EXTRA_ORDS {
            return Err(format!("refused: at most {MAX_EXTRA_ORDS} ords may be added"));
        }
        e.push(base.clone());
        Ok(())
    })?;
    note(format!("ord provider added: {base}"));
    Ok(format!("added: {} providers now", providers().len()))
}

/// The providers a read asks, by name and base. The key is not among what is shown.
#[ic_cdk::query]
pub fn ord_providers() -> Vec<(String, String)> {
    providers().into_iter().map(|p| (p.name, p.base)).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_update_here_is_operator_only_and_nothing_prints_the_key() {
        let src = include_str!("ordnet.rs");
        let updates: Vec<&str> = src.split(concat!("#[ic_cdk::", "update]\npub fn")).skip(1).collect();
        assert_eq!(updates.len(), 3, "set_unisat_key, clear_unisat_key, add_ord_provider");
        for part in &updates {
            let head: String = part.chars().take(200).collect();
            assert!(head.contains("only_operator();"), "unguarded: {head}");
        }
        // the key is read in exactly one place (the outcall), into exactly one header
        let code = src.split(concat!("#[cfg(", "test)]")).next().unwrap();
        assert_eq!(code.matches(concat!("UNISAT_KEY.with(|k| k.borrow()", ".clone())")).count(), 1);
        assert_eq!(code.matches(concat!("value: format!(\"Bearer", " {key}\")")).count(), 1);
        // what set_unisat_key notes and returns is a length, never the value
        let set = updates[0];
        assert!(!set.contains("note(format!(\"{key}") && !set.contains("Ok(key") && set.contains("not shown"));
        // no query returns it
        let queries = code.split(concat!("#[ic_cdk::", "query]")).skip(1).map(|q| q.split("\n}\n").next().unwrap()); // each query's own body
        assert_eq!(queries.filter(|q| q.contains("UNISAT_KEY")).count(), 0);
    }
}
