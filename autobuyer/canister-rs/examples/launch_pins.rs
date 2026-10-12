//! W10 (docs/v9/canister-only-solana.md §12, §16; the ceremony runbook `docs/v9/ceremony-canister-only.md`): the launch pins. From the
//! launch files (`deployments/launch/nft-pilot.json`, `graduation-tiers.json`, `solana-collections.json`) and the unlock order
//! (`coins::unlock::ORDER`), one `coins_pin` argument per collection, ready for `dfx canister call … coins_pin --argument-file`:
//! its index (`ORDER`'s registry: by default the first `LAUNCH_COUNT` = 38, every collection with a pipeline: the 11 Ethereum
//! collections, Bitcoin Frogs, the 12 Solana collections and the 14 Ordinals; each opens when a creator first asks for it, operator
//! decisions 2026-10-12, "the people choose" and "no picking"), names, `T_wX = max(3, ⌈tier / floor⌉)` whole wX
//! (§3.2; the Solana collections' `threshold_wx` as listed), the DBC curve for that threshold in wX (Meteora's own `designCurve`,
//! `solana/tests/q1/design_curve.mjs`, 1e9 supply, 20 % on migration, 50 % migration fee, 6 decimals), the books' value-based parameters
//! (§4.3, `books::value_params`) and its listing kind with the canister's custody address: `EthNft` with `--evm-custody`, `Inscriptions`
//! with `--btc-custody`, `SolNft` with `--sol-custodian` (the canister's own wallet, `sol_address`: a Solana NFT is held by the canister
//! itself). A collection whose custody argument is missing is refused before anything is written. Every pin is checked as the canister
//! checks it (`coins::open::register`). Native only.
//!
//! The Ordinals and Robinhood Chain (2026-10-12, operator decision "no picking"; audit C4-03):
//! - the 14 Ordinals of `nft-pilot.json`'s `inscriptions[]` (`ORDER`'s indices 24–37) are in the default set, each an `Inscriptions`
//!   pin with its own `membersRoot` and the canister's taproot custody (every Inscriptions listing is live by default). `--include-ordinals`
//!   is accepted and does nothing (a note says so): the default has them.
//! - the `chainId` 4663 entries (a Robinhood Chain collection: `robinhood-collections.json`, merged into `nft-pilot.json`'s
//!   `collections[]`) are pinned ONLY with `--include-robinhood`, run after the ceremony has read chain 4663's `evm_chains` entry back
//!   (the canister refuses to open one before: `coins::open::pipeline_ready`). Each is `EthNft { chain_id: 4663 }` with the canister's EVM
//!   address as custody (one address on every EVM chain); its index is its `ORDER` index (38–40) or the entry's `index`, else
//!   `ORDER.len()` (41) onward in file order; its floor and tier come from `graduation-tiers.json`.
//!
//!   cargo run --example launch_pins -- --evm-custody 0x<20 bytes> --btc-custody <bc1p… taproot> --sol-custodian <base58>
//!       [--count 38] [--include-robinhood] [--uri-base https://userook.fun/wx/] [--open-curves 3] [--revenue-sol 20]
//!       [--out ../../deployments/launch/canister-only]
//!   cargo test --example launch_pins      (the selection's tests)
//!
//! Writes `<out>/<index>-<slug>.args` (candid text, one `coins_pin` call each) and `<out>/pins.json` (every number, for review). The
//! custody addresses are the canister's (`evm_address`, `custody_address`, `sol_address` after the install): run this after it.
use std::collections::BTreeMap;
use std::process::Command;

use etch_autobuyer::books::{self, ListingKind};
use etch_autobuyer::coins::state::CollectionPin;
use etch_autobuyer::coins::unlock::{Source, LAUNCH_COUNT, ORDER};
use serde_json::{json, Value};

const LAMPORTS_PER_SOL: u64 = 1_000_000_000;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn load(rel: &str) -> Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deployments/launch").join(rel);
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).expect("json")
}

/// `x.yz` SOL as lamports (the launch files' decimal strings).
fn sol_str(s: &str) -> u64 {
    let (whole, frac) = s.split_once('.').unwrap_or((s, ""));
    let frac: String = format!("{frac:0<9}").chars().take(9).collect();
    whole.parse::<u64>().unwrap() * LAMPORTS_PER_SOL + frac.parse::<u64>().unwrap()
}

/// Meteora's `designCurve` for a threshold of `t_wx` whole wX (the quote has 9 decimals, as SOL has): `(sqrt_start_price, curve)`.
fn design_curve(t_wx: u64) -> (u128, Vec<(u128, u128)>) {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../solana/tests/q1/design_curve.mjs");
    let out = Command::new("node").arg(&script).args(["1000000000", "20", &t_wx.to_string(), "50", "6"]).output().expect("node runs");
    assert!(out.status.success(), "design_curve.mjs: {}", String::from_utf8_lossy(&out.stderr));
    let v: Value = serde_json::from_slice(&out.stdout).expect("the curve as json");
    let u = |x: &Value| x.as_str().unwrap().parse::<u128>().unwrap();
    let curve = v["curve"].as_array().unwrap().iter().map(|c| (u(&c["sqrtPrice"]), u(&c["liquidity"]))).collect();
    (u(&v["sqrtStartPrice"]), curve)
}

fn evm20(s: &str) -> [u8; 20] {
    hex::decode(s.trim().trim_start_matches("0x")).expect("hex").try_into().expect("20 bytes")
}

/// Robinhood Chain's id: an `nft-pilot.json` collection of this `chainId` is an EthNft listing of chain 4663.
const ROBINHOOD_CHAIN: u64 = 4663;

/// Where a pinned collection comes from: an `ORDER` entry (its launch file by `Source`) or a Robinhood Chain collection of `nft-pilot.json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    Order(Source),
    Robinhood,
}

/// One collection to pin: its books index, names and launch file.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Pick {
    index: u32,
    slug: String,
    symbol: String,
    name: String,
    origin: Origin,
}

/// The collections to pin: `ORDER`'s first `count` (never a Robinhood Chain entry), then, with `include_robinhood` only (audit C4-03:
/// after the ceremony's 4663 read-back), every `chainId` 4663 entry of the merged `collections[]` (its `ORDER` index, else its `index`,
/// else 41 onward in file order). A duplicated index or symbol is refused.
fn select(pilot: &Value, count: usize, include_robinhood: bool) -> Result<Vec<Pick>, String> {
    if count > LAUNCH_COUNT {
        return Err(format!("--count {count}: the ceremony pins {LAUNCH_COUNT} at most (the Robinhood Chain collections come with --include-robinhood)"));
    }
    let mut picks: Vec<Pick> = ORDER
        .iter()
        .enumerate()
        .filter(|(i, e)| *i < count && e.source != Source::Robinhood)
        .map(|(i, e)| Pick { index: i as u32, slug: e.slug.into(), symbol: e.symbol.into(), name: e.name.into(), origin: Origin::Order(e.source) })
        .collect();
    let mut next = ORDER.len() as u32;
    for c in pilot["collections"].as_array().ok_or("nft-pilot.json: no collections[]")? {
        if c["chainId"].as_u64() != Some(ROBINHOOD_CHAIN) || !include_robinhood {
            continue;
        }
        let slug = c["slug"].as_str().ok_or("a 4663 collection without a slug")?;
        // one already in ORDER's registry (`Source::Robinhood`) is pinned at its own index, once
        if let Some(i) = ORDER.iter().position(|e| e.slug == slug) {
            if !picks.iter().any(|p| p.slug == slug) {
                let s = |k: &str| c[k].as_str().map(String::from).ok_or(format!("{slug}: no {k}"));
                picks.push(Pick { index: i as u32, slug: slug.into(), symbol: s("symbol")?, name: s("name")?, origin: Origin::Robinhood });
            }
            continue;
        }
        let index = match c["index"].as_u64() {
            Some(i) => i as u32,
            None => {
                next += 1;
                next - 1
            }
        };
        if (index as usize) < ORDER.len() {
            return Err(format!("{slug}: index {index} is ORDER's {} (a Robinhood Chain collection takes 38 or above)", ORDER[index as usize].slug));
        }
        let s = |k: &str| c[k].as_str().map(String::from).ok_or(format!("{slug}: no {k}"));
        picks.push(Pick { index, slug: slug.into(), symbol: s("symbol")?, name: s("name")?, origin: Origin::Robinhood });
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut symbols = std::collections::BTreeSet::new();
    for p in &picks {
        if !seen.insert(p.index) || !symbols.insert(p.symbol.clone()) {
            return Err(format!("{}: index {} or symbol {} is pinned twice", p.slug, p.index, p.symbol));
        }
    }
    Ok(picks)
}

/// `nft-pilot.json` with the Robinhood Chain collections merged into its `collections[]` (they keep their own launch file,
/// `robinhood-collections.json`, measured 2026-10-11): what `main` reads, and what the tests select from (audit C4-14).
fn launch_pilot() -> Value {
    let mut pilot = load("nft-pilot.json");
    pilot["collections"].as_array_mut().unwrap().extend(load("robinhood-collections.json")["collections"].as_array().unwrap().iter().cloned());
    pilot
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let evm = arg(&args, "--evm-custody").map(|s| evm20(&s));
    let btc = arg(&args, "--btc-custody");
    let sol = arg(&args, "--sol-custodian").map(|s| etch_autobuyer::soltx::pubkey(&s).expect("base58"));
    let count: usize = arg(&args, "--count").map(|s| s.parse().unwrap()).unwrap_or(LAUNCH_COUNT);
    if args.iter().any(|a| a == "--include-ordinals") {
        eprintln!("note: --include-ordinals does nothing now: the 14 Ordinals are in the default set (operator decision 2026-10-12, \"no picking\")");
    }
    let include_robinhood = args.iter().any(|a| a == "--include-robinhood");
    let uri_base = arg(&args, "--uri-base").unwrap_or_else(|| "https://userook.fun/wx/".into());
    let open_curves: u64 = arg(&args, "--open-curves").map(|s| s.parse().unwrap()).unwrap_or(3);
    let revenue: u64 = arg(&args, "--revenue-sol").map(|s| s.parse().unwrap()).unwrap_or(20) * LAMPORTS_PER_SOL;
    let out = arg(&args, "--out").unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deployments/launch/canister-only").to_string_lossy().into());

    let pilot = launch_pilot();
    let tiers = load("graduation-tiers.json");
    let solana = load("solana-collections.json");
    let eth: BTreeMap<String, &Value> = pilot["collections"].as_array().unwrap().iter().map(|c| (c["slug"].as_str().unwrap().to_string(), c)).collect();
    let mut ord: BTreeMap<String, &Value> = pilot["inscriptions"].as_array().unwrap().iter().map(|c| (c["slug"].as_str().unwrap().to_string(), c)).collect();
    ord.insert(pilot["bitcoin"]["slug"].as_str().unwrap().to_string(), &pilot["bitcoin"]);
    let solc: BTreeMap<String, &Value> = solana["collections"].as_array().unwrap().iter().map(|c| (c["key"].as_str().unwrap().to_string(), c)).collect();

    let picks = select(&pilot, count, include_robinhood).unwrap_or_else(|e| {
        eprintln!("refused: {e}; no pin written");
        std::process::exit(2);
    });

    // every custody the pins need is given, or nothing is written (a Solana pin without `--sol-custodian` is never half a set)
    for p in &picks {
        let missing = match p.origin {
            Origin::Order(Source::Solana) if sol.is_none() => Some("--sol-custodian (the canister's `sol_address`: it holds the Solana NFTs itself)"),
            Origin::Order(Source::Ethereum) | Origin::Robinhood if evm.is_none() && eth.contains_key(&p.slug) => Some("--evm-custody (the canister's `evm_address`)"),
            Origin::Order(Source::Bitcoin | Source::Ordinals) if btc.is_none() => Some("--btc-custody (the canister's `custody_address`)"),
            _ => None,
        };
        if let Some(arg) = missing {
            eprintln!("refused: {} needs {arg}; no pin written", p.slug);
            std::process::exit(2);
        }
    }
    std::fs::create_dir_all(&out).unwrap();

    let mut review = vec![];
    let mut total_t = 0u64;
    for p in &picks {
        let (index, slug) = (p.index, p.slug.as_str());
        // the floor and the tier at day 1, and the listing kind with the canister's custody: by the collection's launch file, not its position
        let (floor, t_wx, kind) = match p.origin {
            Origin::Order(Source::Ethereum | Source::Bitcoin | Source::Ordinals | Source::Robinhood) | Origin::Robinhood => {
                let t = &tiers["collections"][slug];
                assert!(!t.is_null(), "{slug}: not in graduation-tiers.json");
                let floor = sol_str(t["floor_sol"].as_str().unwrap());
                let tier = t["tier"].as_u64().unwrap() * LAMPORTS_PER_SOL;
                let t_wx = (tier.div_ceil(floor)).max(3);
                let kind = if let Some(c) = eth.get(slug) {
                    let custody = evm.unwrap_or_else(|| panic!("{slug}: an EVM collection needs --evm-custody (the canister's `evm_address`)"));
                    ListingKind::EthNft { chain_id: c["chainId"].as_u64().unwrap(), collection: evm20(c["collection"].as_str().unwrap()), custody }
                } else {
                    let c = ord.get(slug).unwrap_or_else(|| panic!("{slug}: in no launch file"));
                    let root: [u8; 32] = hex::decode(c["membersRoot"].as_str().unwrap().trim_start_matches("0x")).unwrap().try_into().unwrap();
                    let custody = btc.clone().unwrap_or_else(|| panic!("{slug}: an inscriptions collection needs --btc-custody (the canister's `custody_address`)"));
                    ListingKind::Inscriptions { membership_root: root, custody }
                };
                (floor, t_wx, kind)
            }
            Origin::Order(Source::Solana) => {
                let c = solc.get(slug).unwrap_or_else(|| panic!("{slug}: not in solana-collections.json"));
                let floor = (c["floor_sol"].as_f64().unwrap() * LAMPORTS_PER_SOL as f64).round() as u64;
                let t_wx = c["threshold_wx"].as_u64().unwrap().max(3);
                let custodian = sol.unwrap_or_else(|| panic!("{slug}: a Solana collection needs --sol-custodian (the canister's `sol_address`): no pin written without it"));
                (floor, t_wx, ListingKind::SolNft { collection: etch_autobuyer::soltx::pubkey(c["collection_key"].as_str().unwrap()).unwrap(), custodian })
            }
        };
        let params = books::value_params(floor, t_wx, open_curves, revenue).expect("the value-based parameters");
        let (sqrt_start_price, curve) = design_curve(t_wx);
        let pin = CollectionPin { index, slug: slug.into(), name: p.name.clone(), symbol: p.symbol.clone(), uri: format!("{uri_base}{slug}.json"), t_wx,
            sqrt_start_price, curve: curve.clone(), kind: kind.clone(), params: params.clone(), wx_mint: None, config: None };
        // as the canister checks it
        etch_autobuyer::coins::open::register(pin.clone()).unwrap_or_else(|e| panic!("{}: {e}", pin.slug));
        // the candid text of the call, with the field names
        let bytes = candid::encode_one(&pin).unwrap();
        let mut env = candid::types::internal::TypeContainer::new();
        let ty = env.add::<CollectionPin>();
        let text = candid::IDLArgs::from_bytes_with_types(&bytes, &env.env, &[ty]).unwrap().to_string();
        std::fs::write(format!("{out}/{index:02}-{slug}.args"), format!("{text}\n")).unwrap();
        total_t += t_wx;
        review.push(json!({ "index": index, "slug": slug, "symbol": p.symbol, "name": p.name, "floor_lamports": floor, "t_wx": t_wx,
            "sqrt_start_price": sqrt_start_price.to_string(), "curve_points": curve.len(), "mint_per_hour": params.mint_per_hour,
            "max_public_eth_backed": params.max_public_eth_backed, "max_eth_backed": params.max_eth_backed,
            "label": etch_autobuyer::coins::open::listing_label(&kind), "kind": format!("{kind:?}") }));
        eprintln!("{index:02} {slug:<12} floor {:>8.3} SOL  T_wX {t_wx:>3}  mint/h {:>4} wX  public cap {:>5} wX  float cap {:>5} wX",
            floor as f64 / 1e9, params.mint_per_hour / books::ONE, params.max_public_eth_backed / books::ONE, params.max_eth_backed / books::ONE);
    }
    let n = picks.len();
    std::fs::write(format!("{out}/pins.json"), serde_json::to_string_pretty(&json!({
        "_": "W10 launch pins (examples/launch_pins.rs): one coins_pin call per <index>-<slug>.args; the canister opens a collection when a creator first asks for it (coins_request_open), ≈ 0.023 SOL from K_main's float each; every Inscriptions listing is live by default; the Robinhood Chain collections only with --include-robinhood, after the ceremony read chain 4663 back",
        "open_curves": open_curves, "revenue_target_lamports": revenue, "uri_base": uri_base, "collections": review })).unwrap()).unwrap();
    eprintln!("{n} pins written to {out} (T_wX in all: {total_t} wX); each opens on its first request (coins_request_open), ≈ {:.4} SOL from K_main each ({:.3} SOL were all {n} to open)",
        etch_autobuyer::coins::unlock::OPEN_COST as f64 / 1e9, n as f64 * etch_autobuyer::coins::unlock::OPEN_COST as f64 / 1e9);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pilot_with(extra: Value) -> Value {
        let mut p = launch_pilot();
        p["collections"].as_array_mut().unwrap().extend(extra.as_array().unwrap().iter().cloned());
        p
    }

    /// Audit C4-03, C4-14: the default, run on the files `main` reads (the Robinhood Chain file merged), is the 38 with a pipeline:
    /// ORDER's first 38 at their own index, the 14 Ordinals among them, and no Robinhood Chain collection.
    #[test]
    fn the_default_is_the_38_and_never_a_robinhood_collection() {
        let pilot = launch_pilot();
        assert_eq!(pilot["collections"].as_array().unwrap().iter().filter(|c| c["chainId"] == ROBINHOOD_CHAIN).count(), 3, "the merged files");
        let p = select(&pilot, LAUNCH_COUNT, false).unwrap();
        assert_eq!(p.len(), 38);
        assert!(p.iter().enumerate().all(|(i, x)| x.index == i as u32 && x.origin != Origin::Robinhood && x.origin != Origin::Order(Source::Robinhood)));
        assert_eq!(p.iter().find(|x| x.slug == "bitcoin-frogs").unwrap().index, 4);
        let ords: Vec<_> = p.iter().filter(|x| x.origin == Origin::Order(Source::Ordinals)).collect();
        assert_eq!(ords.len(), 14);
        assert_eq!((ords[0].slug.as_str(), ords[0].index), ("omb", 24));
        for o in &ords {
            let c = pilot["inscriptions"].as_array().unwrap().iter().find(|c| c["slug"] == o.slug.as_str()).expect("in inscriptions[]");
            assert_eq!(hex::decode(c["membersRoot"].as_str().unwrap().trim_start_matches("0x")).unwrap().len(), 32);
        }
        // a count beyond the 38 never reaches ORDER's Robinhood entries
        assert!(select(&pilot, LAUNCH_COUNT + 1, false).unwrap_err().contains("--include-robinhood"));
        assert_eq!(select(&pilot, 2, false).unwrap().len(), 2);
    }

    /// Audit C4-03: `--include-robinhood` (after the ceremony's 4663 read-back) adds the three at ORDER's indices 38–40.
    #[test]
    fn include_robinhood_adds_the_three_at_their_order_indices() {
        let p = select(&launch_pilot(), LAUNCH_COUNT, true).unwrap();
        assert_eq!(p.len(), 41);
        let r: Vec<_> = p.iter().filter(|x| x.origin == Origin::Robinhood).map(|x| (x.slug.as_str(), x.index)).collect();
        assert_eq!(r, vec![("stonkbrokers", 38), ("chainmancers", 39), ("rekttradooors", 40)]);
    }

    #[test]
    fn a_4663_collection_is_pinned_after_the_order() {
        let rh = json!([
            { "slug": "cashcat", "name": "Wrapped CASHCAT", "symbol": "wCASHCAT", "chainId": 4663, "collection": "0x0000000000000000000000000000000000004663" },
        ]);
        let p = select(&pilot_with(rh.clone()), LAUNCH_COUNT, false).unwrap();
        assert_eq!(p.len(), 38, "not without --include-robinhood");
        let p = select(&pilot_with(rh), LAUNCH_COUNT, true).unwrap();
        assert_eq!(p.len(), 42);
        let r: Vec<_> = p.iter().filter(|x| x.origin == Origin::Robinhood).map(|x| (x.slug.as_str(), x.index)).collect();
        // the three of ORDER's registry are pinned at their own indices; an unregistered 4663 collection takes the next after ORDER (41)
        assert_eq!(r, vec![("stonkbrokers", 38), ("chainmancers", 39), ("rekttradooors", 40), ("cashcat", 41)]);
        let kind = ListingKind::EthNft { chain_id: ROBINHOOD_CHAIN, collection: [0; 20], custody: [1; 20] };
        assert_eq!(etch_autobuyer::coins::open::listing_label(&kind), "robinhood");
    }

    #[test]
    fn a_4663_collection_never_takes_an_order_index_or_a_used_symbol() {
        let on_order = json!([{ "slug": "x", "name": "Wrapped X", "symbol": "wX1", "chainId": 4663, "collection": "0x00", "index": 3 }]);
        assert!(select(&pilot_with(on_order), LAUNCH_COUNT, true).unwrap_err().contains("ORDER's smb_gen2"));
        let dup = json!([{ "slug": "x", "name": "Wrapped X", "symbol": "wBOREDAPE", "chainId": 4663, "collection": "0x00" }]);
        assert!(select(&pilot_with(dup), LAUNCH_COUNT, true).unwrap_err().contains("pinned twice"));
        assert!(select(&launch_pilot(), ORDER.len() + 1, false).is_err());
    }
}
