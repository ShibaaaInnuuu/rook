//! Run-time pins and launch checks of `rook-ceremony` (audit A6, docs/v9/audit-2026-10/a6-launch-ops.md):
//!
//! - A6-05 / A3-02: the COMPLETE D-e (collection) and D-g (genesis ROOK) `ConfigParameters` are compared by BYTE EQUALITY of their
//!   Borsh encoding with the canonical encoding committed in `ops/solana/day1-pins.json` (every field: fee schedule, curve, sqrt
//!   start price, migration_fee_option, migrated pool fee and its collect_fee_mode == 0, pool_creation_fee 0, token_update_authority,
//!   vesting, LP percentages, supply…). The accounts (fee_claimer = leftover_receiver) are built by the tool, not read from the file.
//! - the listing order is compared with the pinned order AND with the order derived from `deployments/launch/nft-pilot.json`
//!   (compiled in): the Ethereum collections (ids 0-10), Bitcoin Frogs (11), then the further Inscriptions listings of
//!   nft-pilot.json `inscriptions[]` (12+, docs/v9/ordinals-collections.md); every entry from Bitcoin Frogs on is an Inscriptions one.
//! - the config account DBC writes is checked byte by byte at the offsets of DBC 0.2.1 (`state/config.rs`; a3-backing.md §4).
//! - the cluster: the RPC's genesis hash against the launch file's `cluster` (mainnet-beta ⇒ Bitcoin Mainnet; localnet ⇒ neither
//!   the mainnet nor the devnet genesis).
//! - ROOK metadata on --send (A6-06).
//!
//! `rook-ceremony pins [--write]` regenerates the pins from `ops/solana/launch-params.mjs --meteora` (a reviewed change: the diff of
//! day1-pins.json is the review).
use super::*;
use sha2::{Digest, Sha256};

pub const PINS_JSON: &str = include_str!("../../../../solana/day1-pins.json");
pub const NFT_PILOT_JSON: &str = include_str!("../../../../../deployments/launch/nft-pilot.json");
/// Each listing's graduation tier (docs/v9/graduation-tiers.md, operator decision 2026-10-10).
pub const TIERS_JSON: &str = include_str!("../../../../../deployments/launch/graduation-tiers.json");
/// The graduation tiers (SOL): a collection config's migration_quote_threshold is one of these, with that tier's pinned curve
/// (launch-params.mjs GRADUATION_TIERS; day1-pins.json `tiers`). The genesis ROOK config stays at 85.
pub const GRADUATION_TIERS_SOL: [u64; 4] = [85, 170, 255, 510];
pub const LAMPORTS_PER_SOL_U64: u64 = 1_000_000_000;
pub const PINS_PATH: &str = "ops/solana/day1-pins.json";

pub fn sha256_hex(b: &[u8]) -> String {
    hex(&Sha256::digest(b))
}
pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The Borsh encoding of `ConfigParameters` (the create_config instruction data after its 8-byte discriminator).
pub fn encode(p: &dbct::ConfigParameters) -> Vec<u8> {
    dbc::client::args::CreateConfig { config_parameters: p.clone() }.data()[8..].to_vec()
}

/// Every `value` leaf of a launch-file parameter object, by dotted path (for naming the fields that drift; the bytes decide).
pub fn leaves(v: &Value, prefix: &str, out: &mut BTreeMap<String, String>) {
    match v {
        Value::Object(o) if o.contains_key("value") => {
            out.insert(prefix.to_string(), o["value"].to_string());
        }
        Value::Object(o) => {
            for (k, x) in o {
                if k == "_" || k == "source" {
                    continue;
                }
                leaves(x, &if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") }, out);
            }
        }
        _ => {}
    }
}

/// The listing order derived from nft-pilot.json and graduation-tiers.json, exactly as launch-params.mjs `listingOrder()` builds it.
pub fn listing_order_from_pilot(pilot: &str, tiers: &str) -> Result<Vec<Value>, String> {
    let p: Value = serde_json::from_str(pilot).map_err(|e| format!("nft-pilot.json: {e}"))?;
    let t: Value = serde_json::from_str(tiers).map_err(|e| format!("graduation-tiers.json: {e}"))?;
    let mut v = vec![];
    for c in p["collections"].as_array().ok_or("nft-pilot.json: collections")? {
        v.push(json!({ "slug": c["slug"], "kind": "EthNft", "name": c["name"], "symbol": c["symbol"], "chain_id": c["chainId"], "collection": c["collection"] }));
    }
    if let Some(b) = p.get("bitcoin").filter(|b| !b.is_null()) {
        v.push(json!({ "slug": b["slug"], "kind": "Inscriptions", "name": b["name"], "symbol": b["symbol"], "members_root": b["membersRoot"] }));
    }
    // docs/v9/ordinals-collections.md: the further Ordinals collections, after Bitcoin Frogs, in file order (ids 12+)
    for b in p.get("inscriptions").and_then(|x| x.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
        v.push(json!({ "slug": b["slug"], "kind": "Inscriptions", "name": b["name"], "symbol": b["symbol"], "members_root": b["membersRoot"] }));
    }
    for (i, e) in v.iter_mut().enumerate() {
        e["listing_id"] = json!(i);
        // docs/v9/graduation-tiers.md: the listing's tier (its day-1 config's threshold), committed with the order
        let slug = e["slug"].as_str().unwrap_or("").to_string();
        let tier = t["collections"][&slug]["tier"].as_u64().filter(|x| GRADUATION_TIERS_SOL.contains(x));
        e["graduation_sol"] = json!(tier.ok_or(format!("graduation-tiers.json: {slug} has no tier (one of {GRADUATION_TIERS_SOL:?})"))?);
    }
    Ok(v)
}

/// A listing's graduation tier in lamports (its `graduation_sol`), refused unless it is one of the tiers.
pub fn listing_threshold(l: &Value) -> Result<u64, String> {
    let t = l["graduation_sol"].as_u64().ok_or(format!("listing_order {}: graduation_sol missing", l["slug"]))?;
    if !GRADUATION_TIERS_SOL.contains(&t) {
        return Err(format!("refused: listing_order {}: graduation_sol {t} is not a tier ({GRADUATION_TIERS_SOL:?})", l["slug"]));
    }
    Ok(t * LAMPORTS_PER_SOL_U64)
}

/// `meteora.config_parameters` with `meteora.tiers.<sol>` merged in (leaf by leaf): the collection config of that graduation tier.
pub fn tier_parameters_json(meteora: &Value, sol: u64) -> Result<Value, String> {
    let mut cp = meteora.get("config_parameters").ok_or("meteora.config_parameters missing")?.clone();
    let over = meteora.pointer(&format!("/tiers/{sol}")).ok_or(format!("refused: the launch file has no meteora.tiers.{sol} (docs/v9/graduation-tiers.md)"))?;
    merge_leaves(&mut cp, over);
    Ok(cp)
}

/// Merges `over` into `base` leaf by leaf (a `{ value, source }` object replaces; any other object recurses).
pub fn merge_leaves(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) if !o.contains_key("value") => {
            for (k, v) in o {
                if k == "_" {
                    continue;
                }
                merge_leaves(b.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
        (b, o) => *b = o.clone(),
    }
}

/// Every tier's parameters from the launch file, checked (`check_params`, threshold = the tier) and byte-compared with the pins
/// (`day1-pins.json` tiers.<sol>; the set of tiers must be the pinned set). Returns tier (SOL) → (parameters, sha256).
pub fn check_pinned_tiers(meteora: &Value) -> Result<BTreeMap<u64, (dbct::ConfigParameters, String)>, String> {
    let p = pins()?;
    let have: Vec<u64> = meteora["tiers"].as_object().map(|o| o.keys().filter_map(|k| k.parse().ok()).collect()).unwrap_or_default();
    let mut want: Vec<u64> = p.tiers.keys().copied().collect();
    let mut h = have.clone();
    h.sort();
    want.sort();
    if h != want || want != GRADUATION_TIERS_SOL.to_vec() {
        return Err(format!("refused: meteora.tiers {h:?} is not the pinned set of graduation tiers {want:?} ({PINS_PATH}; tool {GRADUATION_TIERS_SOL:?})"));
    }
    let mut out = BTreeMap::new();
    for t in want {
        let j = tier_parameters_json(meteora, t)?;
        let c = config_parameters(&j)?;
        check_params(&c)?;
        if c.migration_quote_threshold != t * LAMPORTS_PER_SOL_U64 {
            return Err(format!("refused: meteora.tiers.{t}: threshold {} lamports, not {t} SOL", c.migration_quote_threshold));
        }
        let e = encode(&c);
        let (pin, leaves_) = &p.tiers[&t];
        if &e != pin {
            return Err(format!("refused: the {t} SOL tier's config_parameters are not the committed ones ({PINS_PATH} tiers.{t}: sha256 {} ≠ {}): {}",
                sha256_hex(&e), sha256_hex(pin), drift(leaves_, &j)));
        }
        out.insert(t, (c, sha256_hex(&e)));
    }
    Ok(out)
}

pub struct Pins {
    pub collection: Vec<u8>,
    pub rook: Vec<u8>,
    pub collection_leaves: BTreeMap<String, String>,
    pub rook_leaves: BTreeMap<String, String>,
    pub listing_order: Vec<Value>,
    /// graduation tier (SOL) → (Borsh encoding, its value leaves)
    pub tiers: BTreeMap<u64, (Vec<u8>, BTreeMap<String, String>)>,
}

fn unhex(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err("odd hex".into());
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string())).collect()
}

/// The committed pins, with their own sha256 lines verified (a hand edit of the hex without the hash, or the reverse, is refused).
pub fn pins() -> Result<Pins, String> {
    let v: Value = serde_json::from_str(PINS_JSON).map_err(|e| format!("{PINS_PATH}: {e}"))?;
    let get_at = |x: &Value, k: &str| -> Result<(Vec<u8>, BTreeMap<String, String>), String> {
        let h = x["config_parameters_borsh_hex"].as_str().ok_or(format!("{PINS_PATH}: {k}.config_parameters_borsh_hex missing (rook-ceremony pins --write)"))?;
        let b = unhex(h).map_err(|e| format!("{PINS_PATH}: {k}: {e}"))?;
        if x["sha256"].as_str() != Some(&sha256_hex(&b)) {
            return Err(format!("{PINS_PATH}: {k}.sha256 does not match its encoding"));
        }
        let l = x["values"].as_object().map(|o| o.iter().map(|(a, b)| (a.clone(), b.as_str().unwrap_or("").to_string())).collect()).unwrap_or_default();
        Ok((b, l))
    };
    let get = |k: &str| get_at(&v[k], k);
    let (collection, collection_leaves) = get("collection")?;
    let (rook, rook_leaves) = get("genesis_rook")?;
    let mut tiers = BTreeMap::new();
    for (k, x) in v["tiers"].as_object().ok_or(format!("{PINS_PATH}: tiers missing (rook-ceremony pins --write)"))? {
        let t: u64 = k.parse().map_err(|_| format!("{PINS_PATH}: tiers.{k}: not a whole number of SOL"))?;
        tiers.insert(t, get_at(x, &format!("tiers.{k}"))?);
    }
    // the 85 SOL tier IS the collection config (one encoding, pinned twice)
    if tiers.get(&85).map(|t| &t.0) != Some(&collection) {
        return Err(format!("{PINS_PATH}: tiers.85 is not the collection config"));
    }
    let listing_order = v["listing_order"].as_array().ok_or(format!("{PINS_PATH}: listing_order missing"))?.clone();
    if v["listing_order_sha256"].as_str() != Some(&sha256_hex(serde_json::to_string(&listing_order).unwrap().as_bytes())) {
        return Err(format!("{PINS_PATH}: listing_order_sha256 does not match"));
    }
    Ok(Pins { collection, rook, collection_leaves, rook_leaves, listing_order, tiers })
}

fn drift(want: &BTreeMap<String, String>, have_json: &Value) -> String {
    let mut have = BTreeMap::new();
    leaves(have_json, "", &mut have);
    let mut d: Vec<String> = want
        .iter()
        .filter(|(k, v)| have.get(*k) != Some(*v))
        .map(|(k, v)| format!("{k} = {} (pinned {v})", have.get(k).map(String::as_str).unwrap_or("missing")))
        .collect();
    d.extend(have.keys().filter(|k| !want.contains_key(*k)).map(|k| format!("{k} (not pinned)")));
    if d.is_empty() {
        "no leaf differs: the encoding does".into()
    } else {
        d.join("; ")
    }
}

/// A6-05: byte equality of both parameter sets with the committed canonical encodings.
pub fn check_pinned_params(meteora: &Value, params: &dbct::ConfigParameters, rook: &dbct::ConfigParameters) -> Result<(String, String), String> {
    let p = pins()?;
    let (ec, er) = (encode(params), encode(rook));
    if ec != p.collection {
        return Err(format!(
            "refused: meteora.config_parameters is not the committed D-e config ({PINS_PATH}: sha256 {} ≠ {}): {}",
            sha256_hex(&ec),
            sha256_hex(&p.collection),
            drift(&p.collection_leaves, &meteora["config_parameters"])
        ));
    }
    if er != p.rook {
        return Err(format!(
            "refused: the genesis ROOK config_parameters are not the committed D-g config ({PINS_PATH}: sha256 {} ≠ {}): {}",
            sha256_hex(&er),
            sha256_hex(&p.rook),
            drift(&p.rook_leaves, &rook_parameters_json(meteora)?)
        ));
    }
    Ok((sha256_hex(&ec), sha256_hex(&er)))
}

/// A6-05 / A6-02: the launch file's listing_order is exactly the pinned one and the one nft-pilot.json gives (the Ethereum
/// collections, then Bitcoin Frogs, then the further Inscriptions listings: no Ethereum collection after the first Inscriptions one).
pub fn check_listing_order(listings: &[Value]) -> Result<(), String> {
    let p = pins()?;
    let pilot = listing_order_from_pilot(NFT_PILOT_JSON, TIERS_JSON)?;
    if p.listing_order != pilot {
        return Err(format!("refused: {PINS_PATH} listing_order differs from deployments/launch/nft-pilot.json / graduation-tiers.json (regenerate: rook-ceremony pins --write, and review)"));
    }
    for l in listings {
        let t = listing_threshold(l)? / LAMPORTS_PER_SOL_U64;
        if !p.tiers.contains_key(&t) {
            return Err(format!("refused: listing_order {}: the {t} SOL tier is not pinned", l["slug"]));
        }
    }
    if listings.len() != pilot.len() {
        return Err(format!("refused: listing_order has {} entries; nft-pilot.json gives {}", listings.len(), pilot.len()));
    }
    for (i, (have, want)) in listings.iter().zip(&pilot).enumerate() {
        if have != want {
            return Err(format!("refused: listing_order[{i}] is {have}, nft-pilot.json / {PINS_PATH} give {want}"));
        }
    }
    if pilot.last().and_then(|l| l["kind"].as_str()) != Some("Inscriptions") {
        return Err("refused: an Inscriptions listing (Bitcoin Frogs or a later Ordinals collection) must be the last listing".into());
    }
    let first_btc = pilot.iter().position(|l| l["kind"] == "Inscriptions").unwrap_or(pilot.len());
    if pilot[first_btc..].iter().any(|l| l["kind"] != "Inscriptions") || pilot.get(first_btc).map(|l| l["slug"] != "bitcoin-frogs").unwrap_or(true) {
        return Err("refused: the order is the Ethereum collections, Bitcoin Frogs, then the further Inscriptions listings".into());
    }
    Ok(())
}

/// A6-05: `review.reviewed_by` must be filled in by hand before anything is sent.
pub fn check_reviewed(launch: &Value) -> Result<String, String> {
    match launch.pointer("/review/reviewed_by") {
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(s.clone()),
        _ => Err("refused: --send with review.reviewed_by = null: read every line of the launch file, then fill in review.reviewed_by / reviewed_at".into()),
    }
}

/// A6-06: the genesis ROOK metadata is immutable (token_update_authority 1): never sent with an empty or placeholder uri.
/// Accepted hosts (operator decisions 2026-10-10): ipfs://, https://ipfs.io/ipfs/, https://gateway.pinata.cloud/ipfs/, https://arweave.net/,
/// https://res.cloudinary.com/.
/// https://res.cloudinary.com/ (operator decision 2026-10-10: ROOK's metadata is hosted there; never delete or rename those files).
pub const ROOK_URI_PREFIXES: [&str; 5] =
    ["ipfs://", "https://ipfs.io/ipfs/", "https://gateway.pinata.cloud/ipfs/", "https://arweave.net/", "https://res.cloudinary.com/"];
/// The operator's hosted ROOK metadata JSON (2026-10-10): name "ROOK", symbol "ROOK", image …/rook-token/rook.png.
pub const ROOK_URI: &str = "https://res.cloudinary.com/ddwebzwrk/raw/upload/v1791607907/rook-token/rook.json";
pub fn check_rook_metadata_static(name: &str, symbol: &str, uri: &str) -> Result<(), String> {
    if name.trim().is_empty() || symbol.trim().is_empty() {
        return Err("refused: --send with an empty ROOK name or symbol".into());
    }
    if uri.trim().is_empty() {
        return Err("refused: --send needs --rook-uri: the permanently hosted metadata JSON (its \"image\" the hosted rook.png); the mint's metadata is immutable".into());
    }
    let rest = ROOK_URI_PREFIXES.iter().find_map(|p| uri.strip_prefix(p));
    match rest {
        Some(r) if r.len() >= 8 && r.chars().all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c)) => Ok(()),
        _ => Err(format!(
            "refused: --rook-uri {uri:?} is not a permanent content address ({}…; no placeholder)",
            ROOK_URI_PREFIXES.join("…, ")
        )),
    }
}

/// Fetches the metadata JSON (through ipfs.io for ipfs://) and checks name, symbol and an image that resolves.
pub fn check_rook_metadata_hosted(name: &str, symbol: &str, uri: &str) -> Result<(), String> {
    let http = |u: &str| u.strip_prefix("ipfs://").map(|r| format!("https://ipfs.io/ipfs/{r}")).unwrap_or_else(|| u.to_string());
    let get = |u: &str, head: bool| -> Result<Vec<u8>, String> {
        let mut c = Command::new("curl");
        c.args(["-sSfL", "-m", "30", "--proto", "=https"]);
        if head {
            c.args(["-o", "/dev/null", "-w", "%{http_code} %{content_type}"]);
        }
        let o = c.arg(http(u)).output().map_err(|e| format!("curl: {e}"))?;
        if !o.status.success() {
            return Err(format!("refused: {u} does not resolve ({})", String::from_utf8_lossy(&o.stderr).trim()));
        }
        Ok(o.stdout)
    };
    let j: Value = serde_json::from_slice(&get(uri, false)?).map_err(|e| format!("refused: {uri} is not JSON: {e}"))?;
    if j["name"].as_str() != Some(name) || j["symbol"].as_str() != Some(symbol) {
        return Err(format!("refused: {uri} names {} / {}, the launch uses {name} / {symbol}", j["name"], j["symbol"]));
    }
    let img = j["image"].as_str().ok_or(format!("refused: {uri} has no \"image\""))?;
    check_rook_metadata_static(name, symbol, img).map_err(|_| format!("refused: the image {img} is not a permanent content address"))?;
    let h = String::from_utf8_lossy(&get(img, true)?).to_string();
    if !h.contains("image/") {
        return Err(format!("refused: the image {img} answers {h}, not an image"));
    }
    Ok(())
}

/// A6-04 / A6-14: the RPC's cluster is the launch file's; mainnet-beta ⇒ Bitcoin Mainnet; localnet is neither public genesis.
pub fn check_cluster(genesis: &str, launch: &Value) -> Result<(), String> {
    let cluster = launch["cluster"].as_str().unwrap_or("");
    let net = launch.pointer("/init/init_registry/bitcoin_network").or(launch.get("bitcoin_network")).and_then(|v| v.as_str()).unwrap_or("");
    if launch.get("bitcoin_network").and_then(|v| v.as_str()).is_some_and(|b| b != net) {
        return Err("refused: the launch file's bitcoin_network and init.init_registry.bitcoin_network differ".into());
    }
    match cluster {
        "mainnet-beta" if genesis != MAINNET_GENESIS => Err(format!("refused: the launch file names mainnet-beta but the RPC's genesis is {genesis}")),
        "mainnet-beta" if net != "Mainnet" => Err(format!("refused: a mainnet-beta launch file must name Bitcoin Mainnet (it names {net:?}): the registry's network is immutable")),
        "devnet" if genesis != DEVNET_GENESIS => Err(format!("refused: the launch file names devnet but the RPC's genesis is {genesis}")),
        "localnet" if genesis == MAINNET_GENESIS || genesis == DEVNET_GENESIS => Err(format!("refused: the launch file names localnet but the RPC is a public cluster ({genesis})")),
        "mainnet-beta" | "devnet" | "localnet" => Ok(()),
        c => Err(format!("refused: the launch file's cluster {c:?}: mainnet-beta | devnet | localnet")),
    }
}

/// What the config account DBC writes must hold, byte for byte at DBC 0.2.1's offsets (state/config.rs; a3-backing.md §4).
pub struct WantConfig {
    pub claimer: Pubkey,
    /// the migration_quote_threshold (lamports): the listing's graduation tier, 85 SOL for ROOK
    pub threshold: u64,
    pub migration_fee_percentage: u8,
    pub creator_trading_fee_percentage: u8,
}
pub fn check_config_bytes(owner: &Pubkey, d: &[u8], w: &WantConfig) -> Result<(), String> {
    let u = |o: usize, n: usize| d[o..o + n].iter().rev().fold(0u128, |a, b| (a << 8) | *b as u128);
    let k = |o: usize| Pubkey::new_from_array(d[o..o + 32].try_into().unwrap());
    if *owner != DBC_ID || d.len() != POOL_CONFIG_LEN || d[..8] != rook_backing::dbc::POOL_CONFIG_DISCRIMINATOR {
        return Err(format!("not a DBC 0.2.1 PoolConfig (owner {owner}, {} bytes)", d.len()));
    }
    let checks: [(bool, &str); 18] = [
        (k(8) == NATIVE_MINT, "quote = wSOL"),
        (k(40) == w.claimer, "fee_claimer"),
        (k(72) == w.claimer, "leftover_receiver = fee_claimer"),
        ((u(104, 8), u(112, 8), u(120, 8), u(128, 2), u(130, 1)) == (500_000_000, 10, 8_166_666, 60, 0), "base fee (50 % → 1 % over 10 min, linear)"),
        (d[184] == 0 && d[200] == 0, "no partner / creator LP vesting"),
        ((d[232], d[233], d[234], d[235], d[237]) == (0, 1, 1, 6, 0), "collect in SOL, DAMM v2, timestamp, 6 decimals, SPL"),
        ((d[239], d[240], d[241], d[242]) == (50, 0, 50, 0), "LP permanently locked 50/50, nothing claimable"),
        (d[243] == 6, "migration_fee_option 6 (Customizable)"),
        (d[244] == 1, "fixed supply"),
        (d[245] == w.creator_trading_fee_percentage, "creator trading-fee share"),
        (d[246] == 1, "token_update_authority 1 (immutable)"),
        ((d[247], d[248]) == (w.migration_fee_percentage, 0), "migration fee / no creator share"),
        (u(264, 8) == w.threshold as u128, "the graduation tier's threshold"),
        ((u(344, 8), u(352, 8)) == (1_000_000_000_000_000, 1_000_000_000_000_000), "supply pre = post = 10^15"),
        (d[360] == 0, "migrated DAMM v2 collect_fee_mode 0 (SOL only; A3-02)"),
        (d[361] == 0, "migrated pool: no dynamic fee"),
        (u(362, 2) == 100, "migrated pool fee 100 bps"),
        (u(368, 8) == 0, "pool_creation_fee 0"),
    ];
    let bad: Vec<&str> = checks.iter().filter(|(ok, _)| !ok).map(|(_, w)| *w).collect();
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!("the config account fails: {}", bad.join(", ")))
    }
}

/// `rook-ceremony pins [--write]`: the canonical encodings from launch-params.mjs --meteora; prints them, or writes the pins file.
pub fn cmd_pins(a: &BTreeMap<String, String>) -> Result<(), String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../..");
    let out = Command::new("node").arg(root.join("ops/solana/launch-params.mjs")).arg("--meteora").output().map_err(|e| format!("node: {e}"))?;
    if !out.status.success() {
        return Err(format!("launch-params.mjs --meteora: {}", String::from_utf8_lossy(&out.stderr)));
    }
    let v: Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    let m = &v["meteora"];
    let c = config_parameters(&m["config_parameters"])?;
    check_params(&c)?;
    let rj = rook_parameters_json(m)?;
    let r = config_parameters(&rj)?;
    check_rook_params(&r)?;
    let lv = |j: &Value| {
        let mut l = BTreeMap::new();
        leaves(j, "", &mut l);
        Value::Object(l.into_iter().map(|(k, v)| (k, Value::String(v))).collect())
    };
    let rd = |f: &str| fs::read_to_string(root.join(f)).map_err(|e| format!("{f}: {e}"));
    let pilot = listing_order_from_pilot(&rd("deployments/launch/nft-pilot.json")?, &rd("deployments/launch/graduation-tiers.json")?)?;
    let mut tiers = serde_json::Map::new();
    for t in GRADUATION_TIERS_SOL {
        let j = tier_parameters_json(m, t)?;
        let tc = config_parameters(&j)?;
        check_params(&tc)?;
        if tc.migration_quote_threshold != t * LAMPORTS_PER_SOL_U64 {
            return Err(format!("launch-params.mjs tiers.{t}: threshold {}", tc.migration_quote_threshold));
        }
        let e = encode(&tc);
        tiers.insert(t.to_string(), json!({ "config_parameters_borsh_hex": hex(&e), "sha256": sha256_hex(&e), "values": lv(&j) }));
    }
    if m["tiers"].as_object().map(|o| o.keys().filter(|k| *k != "_").count()) != Some(GRADUATION_TIERS_SOL.len()) {
        return Err(format!("launch-params.mjs meteora.tiers is not exactly the tool's tiers {GRADUATION_TIERS_SOL:?}"));
    }
    if v["listing_order"].as_array() != Some(&pilot) {
        return Err("launch-params.mjs listingOrder() and the Rust derivation from nft-pilot.json disagree".into());
    }
    let (ec, er) = (encode(&c), encode(&r));
    let pins = json!({
        "_": "The committed canonical day-1 DBC parameters (decisions D-e, D-g; the graduation tiers of docs/v9/graduation-tiers.md: tiers.<sol>, of which 85 is `collection`) and listing order (with each listing's graduation_sol). rook-ceremony day1 / preflight / list refuse a launch file whose meteora.config_parameters (Borsh-encoded ConfigParameters, every field) or listing_order is not byte-identical to these (audit A6-05). Regenerated only by `rook-ceremony pins --write` from ops/solana/launch-params.mjs and deployments/launch/nft-pilot.json; the diff of this file is the review.",
        "dbc": "Meteora DBC 0.2.1 (dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN, f552f20): ConfigParameters as in process_create_config.rs",
        "collection": { "config_parameters_borsh_hex": hex(&ec), "sha256": sha256_hex(&ec), "values": lv(&m["config_parameters"]) },
        "genesis_rook": { "config_parameters_borsh_hex": hex(&er), "sha256": sha256_hex(&er), "values": lv(&rj) },
        "tiers": Value::Object(tiers),
        "listing_order": pilot,
        "listing_order_sha256": sha256_hex(serde_json::to_string(&pilot).unwrap().as_bytes()),
    });
    let text = serde_json::to_string_pretty(&pins).unwrap() + "\n";
    if a.contains_key("write") {
        fs::write(root.join(PINS_PATH), &text).map_err(|e| e.to_string())?;
        println!("wrote {PINS_PATH}: collection sha256 {}, genesis ROOK sha256 {} (rebuild rook-ceremony to compile them in)", sha256_hex(&ec), sha256_hex(&er));
    } else if text != PINS_JSON {
        return Err(format!("{PINS_PATH} differs from the generator (rook-ceremony pins --write, then review the diff)"));
    } else {
        println!("{PINS_PATH} matches the generator: collection sha256 {}, genesis ROOK sha256 {}", sha256_hex(&ec), sha256_hex(&er));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meteora() -> Option<Value> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../..");
        let out = Command::new("node").arg(root.join("ops/solana/launch-params.mjs")).arg("--meteora").output().ok()?;
        serde_json::from_slice(&out.stdout).ok()
    }

    /// The committed pins are the generator's (launch-params.mjs, nft-pilot.json): `rook-ceremony pins` would rewrite nothing, and
    /// the pinned collection encoding decodes to Q1's tested values (the unit test of ceremony.rs pins those field by field).
    #[test]
    fn the_committed_pins_are_the_generators() {
        let Some(v) = meteora() else { return };
        let p = pins().unwrap();
        let c = config_parameters(&v["meteora"]["config_parameters"]).unwrap();
        let r = config_parameters(&rook_parameters_json(&v["meteora"]).unwrap()).unwrap();
        assert_eq!(encode(&c), p.collection);
        assert_eq!(encode(&r), p.rook);
        assert_eq!(v["listing_order"].as_array().unwrap(), &p.listing_order);
        assert_eq!(listing_order_from_pilot(NFT_PILOT_JSON, TIERS_JSON).unwrap(), p.listing_order);
        let tiers = check_pinned_tiers(&v["meteora"]).unwrap();
        assert_eq!(tiers.keys().copied().collect::<Vec<_>>(), GRADUATION_TIERS_SOL.to_vec());
        assert_eq!(tiers[&85].0.sqrt_start_price, c.sqrt_start_price);
        check_pinned_params(&v["meteora"], &c, &r).unwrap();
        check_listing_order(&p.listing_order).unwrap();
        cmd_pins(&BTreeMap::new()).unwrap();
        // the new wrapped symbols (0c5d4b9) are what is pinned
        let syms: Vec<&str> = p.listing_order.iter().map(|l| l["symbol"].as_str().unwrap()).collect();
        assert_eq!(syms.first(), Some(&"wMILADY"));
        assert_eq!(syms[11], "wBFROG");
        // docs/v9/ordinals-collections.md: the further Ordinals collections, ids 12-16, then 17-25 (2026-10-10, most liquid first)
        assert_eq!(&syms[12..17], &["wNODEMONKE", "wOMB", "wPUPPET", "wBPUNK", "wNATCAT"]);
        assert_eq!(&syms[17..], &["wRUNESTONE", "wWEIRDO", "wCENTS", "wRSIC", "wPIZZA", "wQCAT", "wWOO", "wTAPWIZ", "wBWIZ"]);
        assert!(syms.iter().all(|s| s.len() <= 10), "Metaplex symbols are at most 10 bytes");
        assert_eq!(syms.iter().collect::<std::collections::BTreeSet<_>>().len(), syms.len(), "wX symbols are distinct");
        // docs/v9/graduation-tiers.md: the tiers are pinned with the order (BAYC 510, Pudgy 255, Identity MD and Taproot Wizards 170)
        let tier = |slug: &str| p.listing_order.iter().find(|l| l["slug"] == slug).and_then(|l| l["graduation_sol"].as_u64());
        assert_eq!((tier("bayc"), tier("pudgypenguins"), tier("identitymd"), tier("taproot-wizards"), tier("milady")), (Some(510), Some(255), Some(170), Some(170), Some(85)));
    }

    /// Any one field off (here the migrated pool's collect_fee_mode, A3-02, and the pool creation fee) is refused and named.
    #[test]
    fn a_drifted_field_is_refused_by_the_pin() {
        let Some(mut v) = meteora() else { return };
        v["meteora"]["config_parameters"]["migrated_pool_fee"]["collect_fee_mode"]["value"] = json!(1);
        let c = config_parameters(&v["meteora"]["config_parameters"]).unwrap();
        check_params(&c).unwrap(); // the old run-time check let it through
        let r = config_parameters(&rook_parameters_json(&v["meteora"]).unwrap()).unwrap();
        let e = check_pinned_params(&v["meteora"], &c, &r).unwrap_err();
        assert!(e.contains("migrated_pool_fee.collect_fee_mode = 1 (pinned 0)"), "{e}");
        let Some(mut v) = meteora() else { return };
        v["meteora"]["genesis_rook"]["overrides"]["sqrt_start_price"]["value"] = json!("95072344172433751");
        let c = config_parameters(&v["meteora"]["config_parameters"]).unwrap();
        let r = config_parameters(&rook_parameters_json(&v["meteora"]).unwrap()).unwrap();
        assert!(check_pinned_params(&v["meteora"], &c, &r).unwrap_err().contains("genesis ROOK"));
    }

    #[test]
    fn the_cluster_must_be_the_launch_files() {
        let l = |c: &str, n: &str| json!({ "cluster": c, "bitcoin_network": n, "init": { "init_registry": { "bitcoin_network": n } } });
        assert!(check_cluster(MAINNET_GENESIS, &l("mainnet-beta", "Mainnet")).is_ok());
        assert!(check_cluster(MAINNET_GENESIS, &l("mainnet-beta", "Signet")).unwrap_err().contains("Bitcoin Mainnet"));
        assert!(check_cluster(DEVNET_GENESIS, &l("mainnet-beta", "Mainnet")).is_err());
        assert!(check_cluster(MAINNET_GENESIS, &l("devnet", "Signet")).is_err());
        assert!(check_cluster(DEVNET_GENESIS, &l("devnet", "Signet")).is_ok());
        assert!(check_cluster(MAINNET_GENESIS, &l("localnet", "Signet")).unwrap_err().contains("public cluster"));
        assert!(check_cluster(DEVNET_GENESIS, &l("localnet", "Signet")).is_err());
        assert!(check_cluster("SomeLocalGenesis1111111111111111111111111111", &l("localnet", "Signet")).is_ok());
        assert!(check_cluster(MAINNET_GENESIS, &l("", "Mainnet")).is_err());
        let mut x = l("mainnet-beta", "Mainnet");
        x["init"]["init_registry"]["bitcoin_network"] = json!("Signet");
        assert!(check_cluster(MAINNET_GENESIS, &x).is_err());
    }

    #[test]
    fn the_rook_uri_is_a_permanent_content_address() {
        let ok = ["ipfs://bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi", "https://ipfs.io/ipfs/bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi/rook.json",
            "https://gateway.pinata.cloud/ipfs/QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG", "https://arweave.net/abcdefgh12345678_-", ROOK_URI];
        assert!(ROOK_URI.len() <= 200, "Metaplex MAX_URI_LENGTH");
        for u in ok {
            check_rook_metadata_static("ROOK", "ROOK", u).unwrap();
        }
        for u in ["", "https://…/rook.json", "https://rook.fun/rook.json", "http://ipfs.io/ipfs/bafy12345678", "ipfs://", "ipfs://short", "https://ipfs.io/ipfs/<cid>"] {
            assert!(check_rook_metadata_static("ROOK", "ROOK", u).is_err(), "{u}");
        }
        assert!(check_rook_metadata_static("", "ROOK", ok[0]).is_err());
    }

    /// Network: the operator's hosted ROOK metadata resolves, names ROOK / ROOK, and its image is an image (cargo test -- --ignored).
    #[test]
    #[ignore]
    fn the_hosted_rook_metadata_resolves() {
        check_rook_metadata_hosted("ROOK", "ROOK", ROOK_URI).unwrap();
        assert!(check_rook_metadata_hosted("ROOK", "WRONG", ROOK_URI).is_err());
    }

    #[test]
    fn review_is_required_to_send() {
        assert!(check_reviewed(&json!({ "review": { "reviewed_by": null } })).is_err());
        assert!(check_reviewed(&json!({ "review": { "reviewed_by": "  " } })).is_err());
        assert!(check_reviewed(&json!({})).is_err());
        assert_eq!(check_reviewed(&json!({ "review": { "reviewed_by": "operator" } })).unwrap(), "operator");
    }

    /// The byte check of a written config: a synthetic PoolConfig with D-e's bytes passes; each A3-02 field breaks it.
    #[test]
    fn the_written_config_is_checked_byte_by_byte() {
        let claimer = Pubkey::new_unique();
        let mut d = vec![0u8; POOL_CONFIG_LEN];
        d[..8].copy_from_slice(&rook_backing::dbc::POOL_CONFIG_DISCRIMINATOR);
        d[8..40].copy_from_slice(NATIVE_MINT.as_ref());
        d[40..72].copy_from_slice(claimer.as_ref());
        d[72..104].copy_from_slice(claimer.as_ref());
        let put = |d: &mut Vec<u8>, o: usize, v: u64, n: usize| d[o..o + n].copy_from_slice(&v.to_le_bytes()[..n]);
        put(&mut d, 104, 500_000_000, 8);
        put(&mut d, 112, 10, 8);
        put(&mut d, 120, 8_166_666, 8);
        put(&mut d, 128, 60, 2);
        for (o, v) in [(233, 1), (234, 1), (235, 6), (239, 50), (241, 50), (243, 6), (244, 1), (246, 1), (247, 50)] {
            d[o] = v;
        }
        put(&mut d, 264, 170_000_000_000, 8);
        put(&mut d, 344, 1_000_000_000_000_000, 8);
        put(&mut d, 352, 1_000_000_000_000_000, 8);
        put(&mut d, 362, 100, 2);
        let w = WantConfig { claimer, threshold: 170_000_000_000, migration_fee_percentage: 50, creator_trading_fee_percentage: 0 };
        check_config_bytes(&DBC_ID, &d, &w).unwrap();
        // another tier's threshold is refused
        assert!(check_config_bytes(&DBC_ID, &d, &WantConfig { threshold: 85_000_000_000, ..WantConfig { claimer, threshold: 0, migration_fee_percentage: 50, creator_trading_fee_percentage: 0 } })
            .unwrap_err().contains("threshold"));
        for (o, v, what) in [(360usize, 1u8, "collect_fee_mode"), (368, 1, "pool_creation_fee"), (246, 0, "token_update_authority"), (184, 1, "vesting"), (240, 50, "LP")] {
            let mut x = d.clone();
            x[o] = v;
            assert!(check_config_bytes(&DBC_ID, &x, &w).unwrap_err().contains(what), "{what}");
        }
        let mut x = d.clone();
        x[72..104].copy_from_slice(Pubkey::new_unique().as_ref());
        assert!(check_config_bytes(&DBC_ID, &x, &w).unwrap_err().contains("leftover_receiver"));
        assert!(check_config_bytes(&Pubkey::new_unique(), &d, &w).is_err());
    }
}
