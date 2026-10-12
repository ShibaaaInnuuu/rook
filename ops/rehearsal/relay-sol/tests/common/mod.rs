#![allow(dead_code)]
use serde_json::Value;
use std::path::PathBuf;

pub fn dir(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(sub)
}

pub fn json(path: PathBuf) -> Value {
    serde_json::from_slice(&std::fs::read(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"))).expect("json")
}

/// Every saved quote: `(name, quote bytes, request, fetched_at)`.
pub fn quotes() -> Vec<(String, Vec<u8>, Value, u64)> {
    let manifest = json(dir("quotes").join("manifest.json"));
    let mut out = Vec::new();
    for (name, at) in manifest["fetched_at"].as_object().expect("fetched_at") {
        let body = std::fs::read(dir("quotes").join(format!("{name}.quote.json"))).expect("quote");
        let req = json(dir("quotes").join(format!("{name}.request.json")));
        out.push((name.clone(), body, req, at.as_u64().expect("u64")));
    }
    assert!(out.len() >= 14, "the manifest lists the saved quotes");
    out
}

/// A `getTransaction` / `getAccountInfo` fixture's `result`, or the bare result when saved bare.
pub fn chain(name: &str) -> Value {
    let v = json(dir("chain").join(name));
    match v.get("result") {
        Some(r) if v.get("jsonrpc").is_some() => r.clone(),
        _ => v,
    }
}

pub fn account_data(name: &str) -> Vec<u8> {
    use base64::Engine;
    let v = chain(name);
    base64::engine::general_purpose::STANDARD.decode(v["value"]["data"][0].as_str().expect("base64")).expect("b64")
}
