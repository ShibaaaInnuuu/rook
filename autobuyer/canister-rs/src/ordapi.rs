//! Hosted ordinals APIs, read two at a time (docs/v6/ordinals-apis.md). Rook runs no ord of its own: where an inscription is
//! and what an output carries come from providers, and a fact is believed only when every provider asked agrees on it at the
//! same height. One provider down, one behind, two disagreeing: "unverified", and the flows that ask do what they already do
//! with a missing answer — wait, refuse to sign, judge later.
//!
//! Pure: no network, no clock. The providers' answers are parsed into the same ord-shaped types (`custody::Location`,
//! `custody::Contents`) the flows have always used, so a self-hosted ord is one more `Kind::Ord` provider with another base
//! URL. The outcalls and the transforms that feed this live in `ordnet.rs`.

use crate::custody::{self, Contents, Location, Outpoint};

/// Providers' heights may differ by this many blocks and still be "the same height" (a block boundary, not a stale indexer).
pub const MAX_HEIGHT_GAP: u64 = 1;
/// A read is verified only with this many providers answering it.
pub const MIN_PROVIDERS: usize = 2;

pub const MAINNET_ORD: &str = "https://ordinals.com";
pub const MAINNET_ORDINALS_WALLET: &str = "https://turbo.ordinalswallet.com";
pub const MAINNET_UNISAT: &str = "https://open-api.unisat.io";

/// Which API dialect a provider speaks. The byte is the transform's context, so one transform per read serves every kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, candid::CandidType, serde::Deserialize, serde::Serialize)]
pub enum Kind {
    /// ord's recursive endpoints (`/r/inscription`, `/r/utxo`, `/r/blockheight`): ordinals.com, or an ord of one's own.
    Ord = 1,
    /// Ordinals Wallet's keyless API: a location and a height; no per-output view.
    OrdinalsWallet = 2,
    /// UniSat's Open API: everything, behind a bearer key (docs/v6/ordinals-apis.md §2.1).
    UniSat = 3,
}

impl Kind {
    pub fn byte(self) -> u8 {
        self as u8
    }
    pub fn of_byte(b: u8) -> Option<Kind> {
        match b {
            1 => Some(Kind::Ord),
            2 => Some(Kind::OrdinalsWallet),
            3 => Some(Kind::UniSat),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, candid::CandidType, serde::Deserialize, serde::Serialize)]
pub struct Provider {
    pub kind: Kind,
    pub base: String,
    /// What a log line calls it: `ord`, `ordinalswallet`, `unisat`, or `ord@<host>` for an ord that is not ordinals.com.
    pub name: String,
}

impl Provider {
    pub fn ord(base: &str) -> Provider {
        let base = base.trim_end_matches('/').to_string();
        let name = if base == MAINNET_ORD { "ord".to_string() } else { format!("ord@{}", base.trim_start_matches("https://")) };
        Provider { kind: Kind::Ord, base, name }
    }
    pub fn ordinals_wallet() -> Provider {
        Provider { kind: Kind::OrdinalsWallet, base: MAINNET_ORDINALS_WALLET.into(), name: "ordinalswallet".into() }
    }
    pub fn unisat() -> Provider {
        Provider { kind: Kind::UniSat, base: MAINNET_UNISAT.into(), name: "unisat".into() }
    }
    /// Whether the provider has a per-output view (inscriptions and runes on an outpoint).
    pub fn reads_contents(&self) -> bool {
        self.kind != Kind::OrdinalsWallet
    }
    pub fn url_height(&self) -> String {
        match self.kind {
            Kind::Ord => format!("{}/r/blockheight", self.base),
            Kind::OrdinalsWallet => format!("{}/blockheight", self.base),
            Kind::UniSat => format!("{}/v1/indexer/blockchain/info", self.base),
        }
    }
    pub fn url_inscription(&self, id: &str) -> String {
        match self.kind {
            Kind::Ord => format!("{}/r/inscription/{id}", self.base),
            Kind::OrdinalsWallet => format!("{}/inscription/{id}", self.base),
            Kind::UniSat => format!("{}/v1/indexer/inscription/info/{id}", self.base),
        }
    }
    /// The output's inscriptions (and, for ord, its runes in the same answer). None for a provider without the view.
    pub fn url_output(&self, o: &Outpoint) -> Option<String> {
        match self.kind {
            Kind::Ord => Some(format!("{}/r/utxo/{}", self.base, o.text())),
            Kind::OrdinalsWallet => None,
            Kind::UniSat => Some(format!("{}/v1/indexer/utxo/{}/{}", self.base, hex::encode(o.txid), o.vout)),
        }
    }
    /// A second call for the output's runes, where the first does not carry them.
    pub fn url_runes(&self, o: &Outpoint) -> Option<String> {
        match self.kind {
            Kind::UniSat => Some(format!("{}/v1/indexer/runes/utxo/{}/{}/balance", self.base, hex::encode(o.txid), o.vout)),
            _ => None,
        }
    }
}

/// A URL a provider may be added at: https, a host, nothing that is not a base (no query, no fragment, no credentials).
pub fn check_base(url: &str) -> Result<String, String> {
    let u = url.trim().trim_end_matches('/');
    let host = u.strip_prefix("https://").ok_or("a provider's base is https://<host>[/path]")?;
    if host.is_empty() || host.contains('?') || host.contains('#') || host.contains('@') || host.contains(char::is_whitespace) || u.len() > 200 {
        return Err("a provider's base is https://<host>[/path]: no query, no fragment, no credentials".into());
    }
    Ok(u.to_string())
}

/// Bitcoin mainnet's providers: ord's recursive endpoints at ordinals.com and Ordinals Wallet, keyless; UniSat when the
/// operator has set its key (`ordnet::set_unisat_key`). Two answer a location, so a location verifies without the key; only
/// ord and UniSat answer an output's contents, so contents verify only with it.
pub fn mainnet_providers(with_unisat: bool) -> Vec<Provider> {
    let mut ps = vec![Provider::ord(MAINNET_ORD), Provider::ordinals_wallet()];
    if with_unisat {
        ps.push(Provider::unisat());
    }
    ps
}

// ---- rune names -------------------------------------------------------------------------------------------------------------------

/// A rune's name with the spacers ord prints (`•`) and any other spacer removed, upper-cased: providers print names
/// differently, the letters are the name.
pub fn rune_name(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphabetic()).map(|c| c.to_ascii_uppercase()).collect()
}

fn canonical(mut c: Contents) -> Contents {
    c.inscriptions.sort();
    c.inscriptions.dedup();
    c.runes = c.runes.iter().map(|r| rune_name(r)).collect();
    c.runes.sort();
    c.runes.dedup();
    c
}

// ---- parsers: each provider's reply, as the ord-shaped fact ------------------------------------------------------------------------

fn body_json(who: &str, body: &[u8]) -> Result<serde_json::Value, String> {
    serde_json::from_slice(body).map_err(|_| format!("{who}: not JSON"))
}

fn satpoint_of(who: &str, sp: &str) -> Result<(Outpoint, u64), String> {
    let (out, off) = sp.rsplit_once(':').ok_or_else(|| format!("{who}: a satpoint is txid:vout:offset"))?;
    Ok((Outpoint::parse(out)?, off.parse().map_err(|_| format!("{who}: an unreadable offset"))?))
}

/// ord's `/r/blockheight` and Ordinals Wallet's `/blockheight` (a number as text); UniSat's `/v1/indexer/blockchain/info` (`data.blocks`).
pub fn parse_height(kind: Kind, status: u16, body: &[u8]) -> Result<u64, String> {
    if status != 200 {
        return Err(format!("{kind:?}: HTTP {status} for the height"));
    }
    match kind {
        Kind::Ord | Kind::OrdinalsWallet => std::str::from_utf8(body).ok().and_then(|s| s.trim().parse().ok()).ok_or_else(|| format!("{kind:?}: an unreadable height")),
        Kind::UniSat => unisat_data(body)?.and_then(|d| d.get("blocks").and_then(|b| b.as_u64())).ok_or_else(|| "unisat: no height".to_string()),
    }
}

/// Ordinals Wallet's `/inscription/<id>`: `satpoint` and `value`, like ord's. `Ok(None)`: 404.
pub fn parse_ow_inscription(status: u16, body: &[u8], id: &str) -> Result<Option<Location>, String> {
    if status == 404 {
        return Ok(None);
    }
    if status != 200 {
        return Err(format!("ordinalswallet: HTTP {status}"));
    }
    let v = body_json("ordinalswallet", body)?;
    if v.get("id").and_then(|x| x.as_str()) != Some(id) {
        return Err("ordinalswallet: the answer is about a different inscription".into());
    }
    let sp = v.get("satpoint").and_then(|x| x.as_str()).ok_or("ordinalswallet: no satpoint")?;
    let (output, offset) = satpoint_of("ordinalswallet", sp)?;
    let value = v.get("value").and_then(|x| x.as_u64()).ok_or("ordinalswallet: no value")?;
    if offset >= value {
        return Err("ordinalswallet: the inscribed sat's offset is past the output's value".into());
    }
    Ok(Some(Location { output, offset, value }))
}

/// UniSat's envelope: HTTP 200 and `code` 0, or nothing is read (a quota or key error is `code` -2004 under HTTP 403, and is
/// never "not found"). `Ok(None)`: `data` null.
fn unisat_data(body: &[u8]) -> Result<Option<serde_json::Value>, String> {
    let v = body_json("unisat", body)?;
    if v.get("code").and_then(|c| c.as_i64()) != Some(0) {
        return Err(format!("unisat: code {}", v.get("code").map(|c| c.to_string()).unwrap_or_else(|| "?".into())));
    }
    Ok(match v.get("data") {
        None | Some(serde_json::Value::Null) => None,
        Some(d) => Some(d.clone()),
    })
}

/// UniSat's `/v1/indexer/inscription/info/<id>` (shape per its swagger, UNVERIFIED against a keyed call): `utxo.{txid,vout,satoshi}`
/// and `offset`. `Ok(None)`: `data` null.
pub fn parse_unisat_inscription(status: u16, body: &[u8], id: &str) -> Result<Option<Location>, String> {
    if status != 200 {
        return Err(format!("unisat: HTTP {status}"));
    }
    let Some(d) = unisat_data(body)? else { return Ok(None) };
    if d.get("inscriptionId").and_then(|x| x.as_str()) != Some(id) {
        return Err("unisat: the answer is about a different inscription".into());
    }
    let u = d.get("utxo").filter(|u| u.is_object()).ok_or("unisat: no utxo")?;
    let txid = u.get("txid").and_then(|x| x.as_str()).ok_or("unisat: no txid")?;
    let vout = u.get("vout").and_then(|x| x.as_u64()).ok_or("unisat: no vout")?;
    let output = Outpoint::parse(&format!("{txid}:{vout}"))?;
    let value = u.get("satoshi").and_then(|x| x.as_u64()).ok_or("unisat: no satoshi")?;
    let offset = d.get("offset").and_then(|x| x.as_u64()).ok_or("unisat: no offset")?;
    if offset >= value {
        return Err("unisat: the inscribed sat's offset is past the output's value".into());
    }
    Ok(Some(Location { output, offset, value }))
}

/// UniSat's `/v1/indexer/utxo/<txid>/<vout>`: the inscriptions and the value; a spent output reads as `None`, as ord's 404 does.
/// The runes come from `parse_unisat_runes`; this leaves `runes` empty.
pub fn parse_unisat_utxo(status: u16, body: &[u8]) -> Result<Option<Contents>, String> {
    if status != 200 {
        return Err(format!("unisat: HTTP {status}"));
    }
    let Some(d) = unisat_data(body)? else { return Ok(None) };
    if d.get("isSpent").and_then(|x| x.as_bool()) == Some(true) {
        return Ok(None);
    }
    let inscriptions = d.get("inscriptions").and_then(|x| x.as_array()).ok_or("unisat: no inscriptions field")?
        .iter().map(|x| x.get("inscriptionId").and_then(|i| i.as_str()).map(str::to_string).ok_or("unisat: an inscription with no id"))
        .collect::<Result<Vec<_>, _>>()?;
    for i in &inscriptions {
        custody::check_inscription_id(i)?;
    }
    let value = d.get("satoshi").and_then(|x| x.as_u64()).ok_or("unisat: no satoshi")?;
    Ok(Some(Contents { inscriptions, runes: Vec::new(), value }))
}

/// UniSat's `/v1/indexer/runes/utxo/<txid>/<vout>/balance`: the runes' names (`spacedRune`, else `rune`).
pub fn parse_unisat_runes(status: u16, body: &[u8]) -> Result<Vec<String>, String> {
    if status != 200 {
        return Err(format!("unisat: HTTP {status}"));
    }
    let Some(d) = unisat_data(body)? else { return Ok(Vec::new()) };
    d.as_array().ok_or("unisat: runes are not a list")?.iter()
        .map(|r| r.get("spacedRune").or_else(|| r.get("rune")).and_then(|x| x.as_str()).map(str::to_string).ok_or_else(|| "unisat: a rune with no name".to_string()))
        .collect()
}

/// Where `kind` says `id` is. `Ok(None)`: the provider does not know it.
pub fn parse_location(kind: Kind, status: u16, body: &[u8], id: &str) -> Result<Option<Location>, String> {
    match kind {
        Kind::Ord => custody::parse_ord_inscription(status, body, id),
        Kind::OrdinalsWallet => parse_ow_inscription(status, body, id),
        Kind::UniSat => parse_unisat_inscription(status, body, id),
    }
}

/// What `kind` says an output carries, in canonical form (ids and spacer-less rune names, sorted). For UniSat, `runes_body`
/// is the second call's reply. `Ok(None)`: not found or spent.
pub fn parse_contents(kind: Kind, status: u16, body: &[u8], runes: Option<(u16, &[u8])>) -> Result<Option<Contents>, String> {
    let c = match kind {
        Kind::Ord => custody::parse_ord_contents(status, body)?,
        Kind::OrdinalsWallet => return Err("ordinalswallet: no per-output view".into()),
        Kind::UniSat => {
            let Some(mut c) = parse_unisat_utxo(status, body)? else { return Ok(None) };
            let (rs, rb) = runes.ok_or("unisat: the runes reply is missing")?;
            c.runes = parse_unisat_runes(rs, rb)?;
            Some(c)
        }
    };
    Ok(c.map(canonical))
}

// ---- the rule: every provider, the same height, the same answer ----------------------------------------------------------------

/// One provider's account: its height before and after the read (an indexer that moved mid-read answered two questions), and the fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer<T> {
    pub provider: String,
    pub height_before: Result<u64, String>,
    pub height_after: Result<u64, String>,
    pub value: Result<T, String>,
}

/// The fact and the height it holds at, or `Err("unverified: …")`. Fewer than [`MIN_PROVIDERS`] answers, any provider failing
/// on any of its three reads, a height that moved during a read, heights more than [`MAX_HEIGHT_GAP`] apart, or two answers
/// that differ: unverified, and the reason names the provider.
pub fn verified<'a, T: PartialEq + std::fmt::Debug>(what: &str, answers: &'a [Answer<T>]) -> Result<(&'a T, u64), String> {
    if answers.len() < MIN_PROVIDERS {
        return Err(format!("unverified: {} of {MIN_PROVIDERS} providers answer {what}", answers.len()));
    }
    let mut heights = Vec::new();
    for a in answers {
        let h0 = a.height_before.as_ref().map_err(|e| format!("unverified: {}: {e}", a.provider))?;
        let h1 = a.height_after.as_ref().map_err(|e| format!("unverified: {}: {e}", a.provider))?;
        if h0 != h1 {
            return Err(format!("unverified: {} moved from {h0} to {h1} during the read of {what}", a.provider));
        }
        a.value.as_ref().map_err(|e| format!("unverified: {}: {e}", a.provider))?;
        heights.push(*h0);
    }
    let (lo, hi) = (*heights.iter().min().unwrap(), *heights.iter().max().unwrap());
    if hi - lo > MAX_HEIGHT_GAP {
        let at: Vec<String> = answers.iter().zip(&heights).map(|(a, h)| format!("{} at {h}", a.provider)).collect();
        return Err(format!("unverified: {} are {} blocks apart on {what}", at.join(", "), hi - lo));
    }
    let first = answers[0].value.as_ref().unwrap();
    for a in &answers[1..] {
        let v = a.value.as_ref().unwrap();
        if v != first {
            return Err(format!("unverified: {} and {} disagree on {what}: {first:?} vs {v:?}", answers[0].provider, a.provider));
        }
    }
    Ok((first, lo))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{}/tests/fixtures/ordapi/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }
    const ID: &str = "3a5b201020e7a3ed48bdc77e25559bbdebd5926904b9b46301c79481476fa703i0";
    const OUT: &str = "579526a20898e3ae9f279241f4fd5c06d9439c1fe2528dc62ddb6e295c2a0ec0:0";
    fn frog() -> Location {
        Location { output: Outpoint::parse(OUT).unwrap(), offset: 0, value: 10_000 }
    }
    fn unisat_inscription(txid: &str, offset: u64) -> Vec<u8> {
        format!(r#"{{"code":0,"msg":"ok","data":{{"inscriptionId":"{ID}","inscriptionNumber":395887,"address":"bc1p","offset":{offset},"contentType":"image/webp","height":780131,"utxo":{{"txid":"{txid}","vout":0,"satoshi":10000,"inscriptions":[{{"inscriptionId":"{ID}","offset":0}}]}}}}}}"#).into_bytes()
    }

    #[test]
    fn the_three_dialects_read_the_same_frog_as_the_same_ord_shaped_location() {
        // ord and Ordinals Wallet: the real answers of 2026-09-23 for Bitcoin Frog #3487; UniSat: per its swagger
        assert_eq!(parse_location(Kind::Ord, 200, &fixture("ord-r-inscription-frog-3487.json"), ID).unwrap(), Some(frog()));
        assert_eq!(parse_location(Kind::OrdinalsWallet, 200, &fixture("ordinalswallet-inscription-frog-3487.json"), ID).unwrap(), Some(frog()));
        assert_eq!(parse_location(Kind::UniSat, 200, &unisat_inscription(&OUT[..64], 0), ID).unwrap(), Some(frog()));
        // not found: ord and Ordinals Wallet answer 404, UniSat a null data
        assert_eq!(parse_location(Kind::Ord, 404, b"inscription x not found", ID).unwrap(), None);
        assert_eq!(parse_location(Kind::OrdinalsWallet, 404, br#"{"error":true,"message":"API error: Status 404 Not Found"}"#, ID).unwrap(), None);
        assert_eq!(parse_location(Kind::UniSat, 200, br#"{"code":0,"msg":"","data":null}"#, ID).unwrap(), None);
        // UniSat's quota and key errors are errors, never "not found"
        let quota = br#"{"code":-2004,"msg":"exceeded the request limit for unauthenticated requests. Please provide Authorization header","data":null}"#;
        assert!(parse_location(Kind::UniSat, 403, quota, ID).unwrap_err().contains("HTTP 403"));
        assert!(parse_location(Kind::UniSat, 200, quota, ID).unwrap_err().contains("code -2004"));
        // an answer about another inscription, or an offset past the value, is refused
        assert!(parse_location(Kind::OrdinalsWallet, 200, &fixture("ordinalswallet-inscription-frog-3487.json"), &ID.replace("i0", "i1")).is_err());
        assert!(parse_location(Kind::UniSat, 200, &unisat_inscription(&OUT[..64], 10_000), ID).is_err());
        assert!(parse_location(Kind::UniSat, 200, b"<html>", ID).unwrap_err().contains("not JSON"));
    }

    #[test]
    fn contents_read_the_same_from_ord_and_unisat_with_rune_names_compared_without_spacers() {
        let ord = parse_contents(Kind::Ord, 200, &fixture("ord-r-utxo-frog-3487.json"), None).unwrap().unwrap();
        assert_eq!(ord, Contents { inscriptions: vec![ID.into()], runes: vec![], value: 10_000 });
        let utxo = format!(r#"{{"code":0,"msg":"ok","data":{{"txid":"{}","vout":0,"satoshi":10000,"inscriptions":[{{"inscriptionId":"{ID}","offset":0}}],"isSpent":false}}}}"#, &OUT[..64]);
        let runes = br#"{"code":0,"msg":"ok","data":[]}"#;
        assert_eq!(parse_contents(Kind::UniSat, 200, utxo.as_bytes(), Some((200, runes))).unwrap().unwrap(), ord);
        let dog = r#"{"code":0,"msg":"ok","data":[{"runeid":"840000:3","rune":"DOGGOTOTHEMOON","spacedRune":"DOG•GO•TO•THE•MOON","amount":"1"}]}"#;
        let with_runes = parse_contents(Kind::UniSat, 200, utxo.as_bytes(), Some((200, dog.as_bytes()))).unwrap().unwrap();
        assert_eq!(with_runes.runes, vec!["DOGGOTOTHEMOON"]);
        let ord_runes = parse_contents(Kind::Ord, 200, r#"{"inscriptions":[],"runes":{"DOG•GO•TO•THE•MOON":{"amount":1,"divisibility":5,"symbol":"🐕"}},"value":546}"#.as_bytes(), None).unwrap().unwrap();
        assert_eq!(ord_runes.runes, vec!["DOGGOTOTHEMOON"]);
        // spent: ord answers 404, UniSat isSpent — the same None
        assert_eq!(parse_contents(Kind::Ord, 404, b"output x not found", None).unwrap(), None);
        assert_eq!(parse_contents(Kind::UniSat, 200, utxo.replace("false", "true").as_bytes(), Some((200, runes))).unwrap(), None);
        // UniSat without its runes reply is not a contents answer; Ordinals Wallet has none
        assert!(parse_contents(Kind::UniSat, 200, utxo.as_bytes(), None).is_err());
        assert!(parse_contents(Kind::OrdinalsWallet, 200, b"{}", None).is_err());
        assert_eq!(rune_name("dog.go.to.the.moon"), "DOGGOTOTHEMOON");
    }

    #[test]
    fn heights_read_from_each_dialect() {
        assert_eq!(parse_height(Kind::Ord, 200, &fixture("ord-r-blockheight.txt")).unwrap(), 968_323);
        assert_eq!(parse_height(Kind::OrdinalsWallet, 200, b"968323\n").unwrap(), 968_323);
        assert_eq!(parse_height(Kind::UniSat, 200, br#"{"code":0,"msg":"ok","data":{"chain":"main","blocks":968323,"headers":968323}}"#).unwrap(), 968_323);
        assert!(parse_height(Kind::Ord, 503, b"").is_err());
        assert!(parse_height(Kind::UniSat, 200, br#"{"code":-1,"msg":"x","data":null}"#).is_err());
    }

    fn ans(p: &str, h: u64, v: Result<Option<Location>, String>) -> Answer<Option<Location>> {
        Answer { provider: p.into(), height_before: Ok(h), height_after: Ok(h), value: v }
    }

    #[test]
    fn the_rule_agree_disagree_one_down_one_behind_moved_or_alone() {
        let h = 968_323;
        // agree: the same location at the same height, from two or three
        assert_eq!(verified("x", &[ans("ord", h, Ok(Some(frog()))), ans("ordinalswallet", h, Ok(Some(frog())))]).unwrap(), (&Some(frog()), h));
        assert_eq!(verified("x", &[ans("ord", h, Ok(Some(frog()))), ans("ordinalswallet", h + 1, Ok(Some(frog()))), ans("unisat", h, Ok(Some(frog())))]).unwrap().1, h);
        // agree on "none"
        assert_eq!(verified("x", &[ans("ord", h, Ok(None)), ans("unisat", h, Ok(None))]).unwrap(), (&None, h));
        // disagree: another outpoint, another offset, or known to one and not the other
        let moved = Location { output: Outpoint::parse(&format!("{}:1", "ab".repeat(32))).unwrap(), ..frog() };
        let e = verified("the frog", &[ans("ord", h, Ok(Some(frog()))), ans("unisat", h, Ok(Some(moved)))]).unwrap_err();
        assert!(e.starts_with("unverified: ord and unisat disagree on the frog"), "{e}");
        assert!(verified("x", &[ans("ord", h, Ok(Some(frog()))), ans("unisat", h, Ok(Some(Location { offset: 1, ..frog() })))]).is_err());
        assert!(verified("x", &[ans("ord", h, Ok(Some(frog()))), ans("unisat", h, Ok(None))]).unwrap_err().contains("disagree"));
        // one down: any of a provider's three reads failing
        assert_eq!(verified("x", &[ans("ord", h, Ok(Some(frog()))), ans("unisat", h, Err("HTTP 503".into()))]).unwrap_err(), "unverified: unisat: HTTP 503");
        let mut no_height = ans("ordinalswallet", h, Ok(Some(frog())));
        no_height.height_before = Err("HTTP 502 for the height".into());
        assert_eq!(verified("x", &[ans("ord", h, Ok(Some(frog()))), no_height]).unwrap_err(), "unverified: ordinalswallet: HTTP 502 for the height");
        // reorg or a stale indexer: heights two apart, or a height that moved during the read
        let e = verified("x", &[ans("ord", h, Ok(Some(frog()))), ans("ordinalswallet", h - 2, Ok(Some(frog())))]).unwrap_err();
        assert_eq!(e, "unverified: ord at 968323, ordinalswallet at 968321 are 2 blocks apart on x");
        let mut moved_on = ans("ord", h, Ok(Some(frog())));
        moved_on.height_after = Ok(h + 1);
        assert_eq!(verified("x", &[moved_on, ans("unisat", h, Ok(Some(frog())))]).unwrap_err(), "unverified: ord moved from 968323 to 968324 during the read of x");
        // alone: one provider is not a check
        assert_eq!(verified("x", &[ans("ord", h, Ok(Some(frog())))]).unwrap_err(), "unverified: 1 of 2 providers answer x");
        assert!(verified::<()>("x", &[]).is_err());
    }

    #[test]
    fn providers_urls_names_and_bases() {
        let o = Outpoint::parse(OUT).unwrap();
        assert_eq!(Provider::ord(MAINNET_ORD).url_inscription(ID), format!("https://ordinals.com/r/inscription/{ID}"));
        assert_eq!(Provider::ord(MAINNET_ORD).url_output(&o).unwrap(), format!("https://ordinals.com/r/utxo/{OUT}"));
        assert_eq!(Provider::ordinals_wallet().url_inscription(ID), format!("https://turbo.ordinalswallet.com/inscription/{ID}"));
        assert_eq!(Provider::ordinals_wallet().url_output(&o), None);
        assert_eq!(Provider::unisat().url_output(&o).unwrap(), format!("https://open-api.unisat.io/v1/indexer/utxo/{}/0", &OUT[..64]));
        assert_eq!(Provider::unisat().url_runes(&o).unwrap(), format!("https://open-api.unisat.io/v1/indexer/runes/utxo/{}/0/balance", &OUT[..64]));
        assert_eq!(Provider::ord("https://ord.example.net/").name, "ord@ord.example.net");
        assert_eq!(mainnet_providers(false).iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["ord", "ordinalswallet"]);
        assert_eq!(mainnet_providers(true).iter().filter(|p| p.reads_contents()).count(), 2);
        assert_eq!(check_base("https://ord.example.net/").unwrap(), "https://ord.example.net");
        for bad in ["http://ord.example.net", "https://", "https://a@b", "https://x/?y", "ordinals.com"] {
            assert!(check_base(bad).is_err(), "{bad}");
        }
        for k in [Kind::Ord, Kind::OrdinalsWallet, Kind::UniSat] {
            assert_eq!(Kind::of_byte(k.byte()), Some(k));
        }
        assert_eq!(Kind::of_byte(0), None);
    }
}
