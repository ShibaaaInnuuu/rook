//! W6: the unlock order (`coins::unlock::ORDER`) against the launch files, so a rename there fails here instead of opening the wrong coin;
//! and the 38 collections the ceremony pins (operator, 2026-10-12: every collection with a pipeline, the Ordinals included; each opens on demand).
use etch_autobuyer::coins::unlock::{is_pinned_at_ceremony, pinned_at_ceremony, Source, LAUNCH_COUNT, ORDER};
use serde_json::Value;

fn load(p: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deployments/launch").join(p);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn every_launch_file_collection_is_in_the_order_once_with_its_symbol_and_name() {
    let pilot = load("nft-pilot.json");
    let sol = load("solana-collections.json");
    // (slug, symbol, name, source) as the launch files spell them
    let mut want: Vec<(String, String, String, Source)> = vec![];
    let s = |v: &Value, k: &str| v[k].as_str().unwrap().to_string();
    for c in pilot["collections"].as_array().unwrap() {
        want.push((s(c, "slug"), s(c, "symbol"), s(c, "name"), Source::Ethereum));
    }
    for c in load("robinhood-collections.json")["collections"].as_array().unwrap() {
        want.push((s(c, "slug"), s(c, "symbol"), s(c, "name"), Source::Robinhood));
    }
    let b = &pilot["bitcoin"];
    want.push((s(b, "slug"), s(b, "symbol"), s(b, "name"), Source::Bitcoin));
    for c in pilot["inscriptions"].as_array().unwrap() {
        want.push((s(c, "slug"), s(c, "symbol"), s(c, "name"), Source::Ordinals));
    }
    for c in sol["collections"].as_array().unwrap() {
        want.push((s(c, "key"), s(c, "symbol"), s(c, "wx_name"), Source::Solana)); // the wX token's on-chain name
    }
    assert_eq!(want.len(), ORDER.len(), "the order lists every launch-file collection");
    for (slug, symbol, name, src) in &want {
        let hits: Vec<_> = ORDER.iter().filter(|e| e.slug == slug).collect();
        assert_eq!(hits.len(), 1, "{slug} is in the order exactly once");
        let e = hits[0];
        assert_eq!((e.symbol, e.name), (symbol.as_str(), name.as_str()), "{slug}");
        assert_eq!(e.source, *src, "{slug}: its launch file");
    }
    // the Solana collections are in the file's order (two of them at launch, the ten others after the Ethereum collections)
    let sol_order: Vec<&str> = ORDER.iter().filter(|e| e.source == Source::Solana).map(|e| e.slug).collect();
    let file: Vec<String> = sol["collections"].as_array().unwrap().iter().map(|c| s(c, "key")).collect();
    assert_eq!(sol_order, file.iter().map(|x| x.as_str()).collect::<Vec<_>>());
    // the Ordinals are in the file's order but OMB, which goes first
    let ord: Vec<&str> = ORDER.iter().filter(|e| e.source == Source::Ordinals).map(|e| e.slug).collect();
    let mut file_ord: Vec<String> = pilot["inscriptions"].as_array().unwrap().iter().map(|c| s(c, "slug")).collect();
    file_ord.retain(|x| x != "omb");
    assert_eq!(ord[0], "omb");
    assert_eq!(ord[1..], file_ord.iter().map(|x| x.as_str()).collect::<Vec<_>>()[..]);
}

#[test]
fn the_ceremony_pins_every_collection_with_a_pipeline_and_each_has_what_its_pin_needs() {
    let pilot = load("nft-pilot.json");
    let tiers = load("graduation-tiers.json");
    let sol = load("solana-collections.json");
    // operator decisions 2026-10-12 ("the people choose", "no picking"): no launch set; the ceremony pins the 11 Ethereum collections,
    // Bitcoin Frogs, the 12 Solana collections and the 14 Ordinals, and each opens when a creator first asks for it (`coins_request_open`)
    assert_eq!(LAUNCH_COUNT, 38);
    assert_eq!(LAUNCH_COUNT, pilot["collections"].as_array().unwrap().len() + 1 + sol["collections"].as_array().unwrap().len()
        + pilot["inscriptions"].as_array().unwrap().len(), "the Robinhood Chain collections (their own file) are pinned after the ceremony's 4663 read-back");
    let head: Vec<&str> = pinned_at_ceremony().iter().take(7).map(|e| e.slug).collect();
    assert_eq!(head, ["bayc", "milady", "mad_lads", "smb_gen2", "bitcoin-frogs", "pudgypenguins", "azuki"], "the registry's head (§16)");
    for (i, e) in pinned_at_ceremony().iter().enumerate() {
        assert!(is_pinned_at_ceremony(i as u32));
        match e.source {
            // an Ethereum collection: its ERC-721 on mainnet in nft-pilot.json, its tier and day-1 floor in graduation-tiers.json
            Source::Ethereum => {
                let c = pilot["collections"].as_array().unwrap().iter().find(|c| c["slug"] == e.slug).unwrap_or_else(|| panic!("{}: not in nft-pilot.json", e.slug));
                assert_eq!(c["chainId"], 1, "{}", e.slug);
                assert!(!tiers["collections"][e.slug].is_null(), "{}: not in graduation-tiers.json", e.slug);
            }
            // Bitcoin Frogs: nft-pilot.json's `bitcoin` (its members root and Ordinals Wallet slug), tiered like the others
            Source::Bitcoin => {
                assert_eq!(pilot["bitcoin"]["slug"], e.slug);
                assert!(pilot["bitcoin"]["membersRoot"].is_string());
                assert!(!tiers["collections"][e.slug].is_null(), "{}: not in graduation-tiers.json", e.slug);
            }
            // a Solana collection in solana-collections.json: its collection key, Magic Eden symbol, Tensor slug, floor and T_wX
            Source::Solana => {
                let c = sol["collections"].as_array().unwrap().iter().find(|c| c["key"] == e.slug).unwrap_or_else(|| panic!("{}: not in solana-collections.json", e.slug));
                assert!(["pnft", "legacy"].contains(&c["standard"].as_str().unwrap()), "{}", e.slug);
                assert!(c["collection_key"].is_string() && c["magic_eden_symbol"].is_string() && c["tensor_slug"].is_string(), "{}", e.slug);
                assert!(c["floor_sol"].as_f64().unwrap() > 0.0, "{}", e.slug);
                assert!(c["threshold_wx"].as_u64().unwrap() >= 3, "{}", e.slug);
            }
            // an Ordinals collection: nft-pilot.json's `inscriptions[]` (its members root), tiered like the others; live by default
            Source::Ordinals => {
                let c = pilot["inscriptions"].as_array().unwrap().iter().find(|c| c["slug"] == e.slug).unwrap_or_else(|| panic!("{}: not in inscriptions[]", e.slug));
                assert!(c["membersRoot"].is_string(), "{}", e.slug);
                assert!(!tiers["collections"][e.slug].is_null(), "{}: not in graduation-tiers.json", e.slug);
            }
            Source::Robinhood => panic!("{}: the Robinhood Chain collections are pinned after the ceremony's 4663 read-back", e.slug),
        }
    }
    let n = |s: Source| pinned_at_ceremony().iter().filter(|e| e.source == s).count();
    assert_eq!((n(Source::Ethereum), n(Source::Bitcoin), n(Source::Solana), n(Source::Ordinals), n(Source::Robinhood)), (11, 1, 12, 14, 0));
    assert_eq!(ORDER[2].slug, "mad_lads");
    assert_eq!(sol["collections"][0]["threshold_wx"], 11);
    assert_eq!(sol["collections"][1]["threshold_wx"], 7);
}
