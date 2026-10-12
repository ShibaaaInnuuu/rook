//! `rook-ceremony list`: the listing ceremony after the deploy (audit A6-02, A6-15). Day 1 made each collection's DBC config pay
//! `claimer(listing(i))` for a listing id that did not exist yet; rook_core gives ids in `list_collection` order and every listing
//! field that matters is immutable. So this tool:
//!
//! - runs `preflight` first (cluster, the pinned + reviewed launch file, the keys = day 1's, the day-1 on-chain configs (one per listing) pay the claimers
//!   recomputed from the ids);
//! - lists STRICTLY in the launch file's `listing_order` (which preflight compared with the pins and nft-pilot.json), asserting
//!   `Registry.listing_count == i` before each `list_collection`, and reads every field of the listing back (id, kind with chain /
//!   collection or membership root, name, custody, custodian, poster); a listing that exists with any other field stops everything;
//! - creates the wrapper, the floor module (the launch file's `init.init_module.params`) and adds it to the treasury;
//! - registers the rook_backing partner and its day-1 config BY SLUG from day1-plan.json's addresses (never discovered by
//!   fee_claimer: anyone can create look-alike configs, A6-15), asserting partner.listing == listing(i), partner.claimer ==
//!   claimer(listing(i)) == the config's fee_claimer, and reads the partner back.
//!
//!   rook-ceremony list --launch <launch.json> --plan <keys>/day1/day1-plan.json --custody <custody.json> --rpc <url> [--rpc2 <url>]
//!                      --canister-sol <the canister's Solana address> [--fund-wallet <pubkey>] [--ops <the fueler's Solana hot key>]
//!                      [--owner <keys>/ceremony-payer.json --send [--yes]]
//!
//! Keeper (decision D-i, docs/v9/solana-migration.md §2.9): the canister is the registry keeper. The first owner-signed step after
//! preflight is `propose_roles(keeper = the canister's Solana address)`, which applies at once while Registry.keeper is unset; it is read
//! back (keeper == that key, keeper not paused). The address comes from --canister-sol or the launch file's `canister.sol_address`
//! (both given: they must agree) and must equal custody.json's custodian(s). A different keeper already set is refused (moving it is a
//! 7-day change, propose_roles + accept_roles); the guardian must differ from the keeper and the owner. Never an operator keypair on
//! mainnet: deploy order = install the canister → read its sol_address → deploy-later.sh → list.
//!
//! --ops (docs/v9/fueler-solana.md §5): after init_treasury, the owner's `set_ops(<pubkey>)` makes that key the treasury's operations
//! address (every `allocate` pays it ops_bps, 10 %), read back from the Treasury account. The first one applies at once; replacing one
//! is proposed, waits a day (TREASURY_DELAY) and is applied by anyone with `apply_ops`. Without --ops the treasury keeps ops unset and
//! its operations share joins the reserves pot. Refused: the default key, the treasury or registry PDA, a program id.
//!
//! custody.json (the canister's keys, from its deployment record):
//!   { "custodian": "<the canister's Ed25519 Solana key>", "poster": "<the EthNft floor poster; default = custodian>",
//!     "eth_custody": "0x… (the canister's threshold-ECDSA address)", "btc_custody": "bc1p… (the canister's taproot address)",
//!     "per_slug": { "<slug>": { any of the four, overriding } } }
//!
//! Without --send each step not yet on chain is simulated while its predecessors exist, and listed otherwise; nothing is sent. The
//! registry owner signs and pays: during the setup that is the ceremony key (`init` names it, day1-plan.json's `ceremony_payer`), so
//! --owner is <keys>/ceremony-payer.json. The fund wallet (the operator's Phantom / Ledger wallet) is NEVER a key file: --owner equal
//! to it is refused, and once the registry belongs to the fund wallet (after `propose-owner` + the wallet's `accept-owner`) --send is
//! refused (a dry run still reads every step back). The handover (ceremony_owner.rs) comes after the whole ceremony.
use super::*;
use anchor_lang::AccountDeserialize;
use rook_core::{
    instructions::ListArgs,
    seeds::{ITEMS, LISTING, MODULE, MODULE_VAULT, MODULE_WX, WRAPPER, WX_MINT},
    metadata::{WxMetadata, METADATA_SEED, MPL_TOKEN_METADATA_ID},
    state::ListingKind,
    Params,
};

pub struct ColPdas {
    pub listing: Pubkey,
    pub wrapper: Pubkey,
    pub wx_mint: Pubkey,
    pub item_set: Pubkey,
    pub module: Pubkey,
    pub vault: Pubkey,
    pub module_wx: Pubkey,
}

pub fn col_pdas(core: &Pubkey, id: u32) -> ColPdas {
    let p = |s: &[&[u8]]| Pubkey::find_program_address(s, core).0;
    let listing = p(&[LISTING, &id.to_le_bytes()]);
    let wrapper = p(&[WRAPPER, listing.as_ref()]);
    let module = p(&[MODULE, listing.as_ref()]);
    ColPdas {
        listing,
        wrapper,
        wx_mint: p(&[WX_MINT, wrapper.as_ref()]),
        item_set: p(&[ITEMS, wrapper.as_ref()]),
        module,
        vault: p(&[MODULE_VAULT, module.as_ref()]),
        module_wx: p(&[MODULE_WX, module.as_ref()]),
    }
}

fn u64s(v: &Value) -> Result<u64, String> {
    match v {
        Value::String(s) => s.parse().map_err(|_| format!("{s}: not a u64")),
        Value::Number(n) => n.as_u64().ok_or(format!("{n}: not a u64")),
        _ => Err(format!("{v}: not a u64")),
    }
}

pub fn module_params(launch: &Value) -> Result<Params, String> {
    let p = &launch["init"]["init_module"]["params"];
    let g = |k: &str| -> Result<u64, String> { u64s(&p[k]).map_err(|e| format!("launch init.init_module.params.{k}: {e}")) };
    let h = |k: &str| -> Result<u16, String> { u16::try_from(g(k)?).map_err(|_| format!("launch init.init_module.params.{k}: > u16")) };
    Ok(Params {
        base_spread_bps: h("base_spread_bps")?,
        vol_weight_bps: h("vol_weight_bps")?,
        stale_spread_bps: h("stale_spread_bps")?,
        stress_spread_bps: h("stress_spread_bps")?,
        max_spread_bps: h("max_spread_bps")?,
        max_move_bps: h("max_move_bps")?,
        conv_bps: h("conv_bps")?,
        buffer_bps: h("buffer_bps")?,
        low_confidence_spread_bps: h("low_confidence_spread_bps")?,
        full_confidence_bps: h("full_confidence_bps")?,
        min_confidence_bps: h("min_confidence_bps")?,
        max_items_per_day: h("max_items_per_day")?,
        reserve_bps: h("reserve_bps")?,
        fill_band_bps: h("fill_band_bps")?,
        mint_per_hour: g("mint_per_hour")?,
        burn_per_hour: g("burn_per_hour")?,
        max_public_eth_backed: g("max_public_eth_backed")?,
        max_eth_backed: g("max_eth_backed")?,
        revenue_target: g("revenue_target")?,
    })
}

pub fn desk_params(launch: &Value) -> Result<rook_core::DeskParams, String> {
    let d = &launch["init"]["init_desk"];
    Ok(rook_core::DeskParams {
        bond: u64s(&d["bond"])?,
        min_bond: u64s(&d["min_bond"])?,
        max_bond: u64s(&d["max_bond"])?,
        window: u64s(&d["window"])? as i64,
    })
}

/// The wX mint's Metaplex metadata: name and symbol from the listing order (nft-pilot.json), uri from the custody file's
/// per_slug.<slug>.wx_uri (or "": the owner sets it later with set_wx_metadata; the wX metadata is mutable, unlike ROOK's).
pub fn wx_meta(entry: &Value, custody: &Value) -> Result<WxMetadata, String> {
    let slug = entry["slug"].as_str().unwrap_or("");
    let m = WxMetadata {
        name: entry["name"].as_str().unwrap_or("").to_string(),
        symbol: entry["symbol"].as_str().unwrap_or("").to_string(),
        uri: custody.pointer(&format!("/per_slug/{slug}/wx_uri")).and_then(|v| v.as_str()).unwrap_or("").to_string(),
    };
    if m.name.is_empty() || m.name.len() > 32 || m.symbol.is_empty() || m.symbol.len() > 10 || m.uri.len() > 200 {
        return Err(format!("{slug}: wX metadata {m:?}: name 1-32, symbol 1-10, uri ≤ 200 bytes"));
    }
    Ok(m)
}

fn hex32(s: &str) -> Result<[u8; 32], String> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    if s.len() != 64 {
        return Err(format!("{s}: not 32 bytes of hex"));
    }
    let mut o = [0u8; 32];
    for i in 0..32 {
        o[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|e| e.to_string())?;
    }
    Ok(o)
}

/// The `ListArgs` of listing_order[i] with the custody file (pure; unit-tested).
pub fn list_args(entry: &Value, custody: &Value) -> Result<ListArgs, String> {
    let slug = entry["slug"].as_str().ok_or("listing_order: slug")?;
    let field = |k: &str| -> Option<String> {
        custody.pointer(&format!("/per_slug/{slug}/{k}")).or(custody.get(k)).and_then(|v| v.as_str()).map(String::from)
    };
    let key = |k: &str| -> Result<Pubkey, String> { field(k).ok_or(format!("custody file: {k} missing ({slug})"))?.parse().map_err(|_| format!("custody file: {k} not base58 ({slug})")) };
    let custodian = key("custodian")?;
    let name = entry["name"].as_str().ok_or(format!("listing_order {slug}: name"))?.to_string();
    if name.is_empty() || name.len() > rook_core::state::MAX_NAME {
        return Err(format!("listing_order {slug}: name {name:?} is 1..{} bytes", rook_core::state::MAX_NAME));
    }
    let (kind, custody_s, poster) = match entry["kind"].as_str() {
        Some("EthNft") => {
            let chain_id = entry["chain_id"].as_u64().ok_or(format!("{slug}: chain_id"))?;
            let collection = rook_core::hashing::parse_evm_address(entry["collection"].as_str().unwrap_or("")).ok_or(format!("{slug}: collection"))?;
            let c = field("eth_custody").ok_or(format!("custody file: eth_custody missing ({slug})"))?;
            if rook_core::hashing::parse_evm_address(&c).is_none() {
                return Err(format!("custody file: eth_custody {c} is not an EVM address"));
            }
            let poster = if field("poster").is_some() { key("poster")? } else { custodian };
            (ListingKind::EthNft { chain_id, collection }, c, poster)
        }
        Some("Inscriptions") => {
            let root = hex32(entry["members_root"].as_str().ok_or(format!("{slug}: members_root"))?)?;
            (ListingKind::Inscriptions { membership_root: root }, field("btc_custody").ok_or(format!("custody file: btc_custody missing ({slug})"))?, Pubkey::default())
        }
        k => return Err(format!("listing_order {slug}: kind {k:?}")),
    };
    if custody_s.is_empty() || custody_s.len() > rook_core::state::MAX_CUSTODY {
        return Err(format!("{slug}: custody {custody_s:?}"));
    }
    Ok(ListArgs { kind, name, custody: custody_s, custodian, poster })
}

/// Every field of a listing read back against what was (or would be) listed. Empty = identical.
pub fn listing_diff(l: &rook_core::state::Listing, id: u32, want: &ListArgs) -> Vec<String> {
    let mut d = vec![];
    if l.id != id {
        d.push(format!("id {} ≠ {id}", l.id));
    }
    if l.kind != want.kind {
        d.push(format!("kind {:?} ≠ {:?}", l.kind, want.kind));
    }
    if l.name != want.name {
        d.push(format!("name {:?} ≠ {:?}", l.name, want.name));
    }
    if l.custody != want.custody {
        d.push(format!("custody {} ≠ {}", l.custody, want.custody));
    }
    if l.custodian != want.custodian {
        d.push(format!("custodian {} ≠ {}", l.custodian, want.custodian));
    }
    if l.poster != want.poster {
        d.push(format!("poster {} ≠ {}", l.poster, want.poster));
    }
    d
}

fn simulate(r: &Rpc, ixs: &[Instruction], payer: &Pubkey) -> Result<u64, String> {
    let msg = message(ixs, payer, solana_message::Hash::default());
    let tx = VersionedTransaction { signatures: vec![Default::default(); msg.header().num_required_signatures as usize], message: msg };
    let sim = r.call("simulateTransaction", json!([b64(&tx), { "encoding": "base64", "sigVerify": false, "replaceRecentBlockhash": true, "commitment": "confirmed" }]))?;
    if !sim["value"]["err"].is_null() {
        return Err(format!("refused in simulation: {} {}", sim["value"]["err"], sim["value"]["logs"]));
    }
    Ok(sim["value"]["unitsConsumed"].as_u64().unwrap_or(0))
}

fn get<T: AccountDeserialize>(r: &Rpc, r2: Option<&Rpc>, k: &Pubkey, owner: &Pubkey) -> Result<Option<T>, String> {
    match account2(r, r2, k)? {
        None => Ok(None),
        Some((o, d)) if o == *owner => T::try_deserialize(&mut &d[..]).map(Some).map_err(|e| format!("{k}: {e}")),
        Some((o, _)) => Err(format!("refused: {k} is owned by {o}, not {owner}")),
    }
}

struct Run<'a> {
    r: &'a Rpc,
    owner: Pubkey,
    signer: Option<&'a Keypair>,
    /// dry run: a step not on chain blocks the simulation of the steps after it (they need it)
    blocked: bool,
}

impl Run<'_> {
    /// `done`: already on chain (and checked by the caller). Otherwise simulate, and with a signer send; `then` reads back and checks.
    fn step(&mut self, name: &str, done: bool, ixs: Vec<Instruction>, independent: bool, then: &dyn Fn() -> Result<(), String>) -> Result<(), String> {
        if done {
            println!("  {name}: on chain (read back, checked)");
            return Ok(());
        }
        if self.signer.is_none() && self.blocked && !independent {
            println!("  {name}: to do (simulated with --send once the steps before it land)");
            return Ok(());
        }
        let cu = simulate(self.r, &ixs, &self.owner).map_err(|e| format!("{name}: {e}"))?;
        match self.signer {
            None => {
                println!("  {name}: simulated OK ({cu} CU), not sent");
                if !independent {
                    self.blocked = true;
                }
            }
            Some(k) => {
                let sig = send_one(self.r, &ixs, &[k])?;
                then().map_err(|e| format!("{name}: sent {sig}, but the read-back FAILED: {e}"))?;
                println!("  {name}: SENT {sig} (read back, checked)");
            }
        }
        Ok(())
    }
}

pub fn list(a: &BTreeMap<String, String>) -> Result<(), String> {
    use rook_backing::constants::{PARTNER_SEED, SETTINGS_SEED};
    let need = |k: &str| a.get(k).cloned().ok_or(format!("--{k} is required"));
    let launch = read_json(&need("launch")?)?;
    let plan = read_json(&need("plan")?)?;
    let custody = read_json(&need("custody")?)?;
    let r = Rpc { url: need("rpc")? };
    let r2 = a.get("rpc2").map(|u| Rpc { url: u.clone() });
    let fund: Pubkey = a.get("fund-wallet").map(String::as_str).unwrap_or(FUND_WALLET).parse().map_err(|_| "--fund-wallet")?;
    let core: Pubkey = pk_of(&plan["rook_core"], "rook_core")?;
    let backing: Pubkey = pk_of(&plan["rook_backing"], "rook_backing")?;
    for (flag, id) in [("rook-core", &core), ("rook-backing", &backing)] {
        if a.get(flag).is_some_and(|v| v != &id.to_string()) {
            return Err(format!("refused: --{flag} is not day1-plan.json's {id}"));
        }
    }
    let send = a.contains_key("send");
    let setup: Pubkey = pk_of(&plan["ceremony_payer"], "ceremony_payer")?;
    let owner_kp = a.get("owner").map(|p| read_kp(Path::new(p))).transpose()?;
    if send && owner_kp.is_none() {
        return Err("refused: --send needs --owner <keys>/ceremony-payer.json (the ceremony key owns the registry during the setup)".into());
    }
    if let Some(k) = &owner_kp {
        if k.pubkey() == fund {
            return Err(format!("refused: --owner is the fund wallet {fund}: its key is never a file; the ceremony key signs the listing \
                ceremony and the fund wallet accepts ownership afterwards with one signature (accept-owner)"));
        }
        if k.pubkey() != setup {
            return Err(format!("refused: --owner is {}, not the ceremony key {setup} (day1-plan.json ceremony_payer)", k.pubkey()));
        }
    }
    let canister_sol = canister_sol_of(a, &launch, &custody)?;
    let ops: Option<Pubkey> = a.get("ops").map(|v| v.parse().map_err(|_| format!("--ops {v}: not a base58 Solana address"))).transpose()?;
    let rows = preflight(&r, r2.as_ref(), &launch, &plan, &core, &backing, &fund, None)?;
    let order = launch.pointer("/listing_order/listings").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let network = bitcoin_network(&launch)?;
    let converter = converter_pda(&backing);
    let registry = Pubkey::find_program_address(&[b"registry"], &core).0;
    let treasury = Pubkey::find_program_address(&[b"treasury"], &core).0;
    let desk = Pubkey::find_program_address(&[b"desk"], &core).0;
    let settings = Pubkey::find_program_address(&[SETTINGS_SEED], &backing).0;
    if let Some(o) = ops {
        check_ops(&o, &core, &backing, &treasury, &registry)?;
    }
    let reg_data = account2(&r, r2.as_ref(), &registry)?.ok_or("refused: no registry: run deploy-later.sh (init) first")?;
    let reg0 = check_registry(&reg_data.1, &[setup, fund], network, &converter).map_err(|e| format!("refused: {e}"))?;
    if send && reg0.owner != setup {
        return Err(format!("refused: the registry already belongs to {} (the fund wallet): the listing ceremony runs before the handover; \
            a step left now is signed by the fund wallet itself, never from a key file", reg0.owner));
    }
    let owner = reg0.owner;
    let keeper_done = check_keeper(&reg0, &canister_sol, &fund)?;
    if account2(&r, r2.as_ref(), &settings)?.is_none() {
        return Err("refused: rook_backing has no settings: run deploy-later.sh (init) first".into());
    }
    // every ListArgs before anything is sent (a custody-file error must not stop the ceremony half-way)
    let args: Vec<ListArgs> = order.iter().map(|e| list_args(e, &custody)).collect::<Result<_, _>>()?;
    for e in &order {
        wx_meta(e, &custody)?;
    }
    let params = module_params(&launch)?;
    let dp = desk_params(&launch)?;
    println!("listing ceremony: {} listings in listing_order, registry owner {owner}{}, rook_core {core}, rook_backing {backing}", rows.len(),
        if owner == setup { " (the ceremony key, until the fund wallet accepts)" } else { " (the fund wallet)" });
    for (row, x) in rows.iter().zip(&args) {
        println!("  {:>2} {:<14} {:?} name {:?} custody {} custodian {} poster {}", row.id, row.slug, x.kind, x.name, x.custody, x.custodian, x.poster);
    }
    println!("  registry keeper (the canister, D-i): {canister_sol}");
    match &ops {
        Some(o) => println!("  treasury ops (the fueler's Solana hot key): {o}"),
        None => println!("  treasury ops: unset (no --ops: the operations share joins the reserves pot until set_ops)"),
    }
    if send && !a.contains_key("yes") {
        print!("Send the listing ceremony (irreversible: listing ids and fields are permanent) as the ceremony key {owner}? Type 'list': ");
        std::io::stdout().flush().ok();
        let mut s = String::new();
        std::io::stdin().read_line(&mut s).map_err(|e| e.to_string())?;
        if s.trim() != "list" {
            return Err("not sent".into());
        }
    }
    let ix = |program: Pubkey, data: Vec<u8>, metas: Vec<anchor_lang::prelude::AccountMeta>| Instruction { program_id: program, accounts: metas, data };
    let spl = anchor_spl::token::spl_token::ID;
    let mut run = Run { r: &r, owner, signer: owner_kp.as_ref().filter(|_| send), blocked: false };
    let rr2 = r2.as_ref();

    // D-i: the canister becomes the registry keeper first (immediate while unset), read back with keeper not paused
    let keeper_ok = || -> Result<(), String> {
        let reg: rook_core::state::Registry = get(&r, rr2, &registry, &core)?.ok_or("no registry")?;
        if reg.keeper != canister_sol || reg.keeper_paused {
            return Err(format!("Registry.keeper {} (paused {}), want {canister_sol} unpaused", reg.keeper, reg.keeper_paused));
        }
        Ok(())
    };
    run.step(
        &format!("propose_roles keeper = {canister_sol} (the canister)"),
        keeper_done,
        vec![propose_roles_ix(&core, &owner, &registry, &canister_sol)],
        true,
        &keeper_ok,
    )?;
    run.step(
        "init_treasury",
        account2(&r, rr2, &treasury)?.is_some(),
        vec![ix(core, rook_core::instruction::InitTreasury {}.data(),
            rook_core::accounts::InitTreasury { owner, payer: owner, registry, treasury, system_program: system_program::ID }.to_account_metas(None))],
        true,
        &|| account2(&r, rr2, &treasury)?.map(|_| ()).ok_or("no treasury".into()),
    )?;
    // --ops: set_ops right after init_treasury, owner-signed, read back from the Treasury account. The FIRST operations address applies
    // at once; replacing one is proposed and waits TREASURY_DELAY (a day), then anyone calls apply_ops.
    if let Some(o) = ops {
        let ops_now = || -> Result<Option<(Pubkey, Pubkey, i64)>, String> {
            Ok(get::<rook_core::state::Treasury>(&r, rr2, &treasury, &core)?.map(|t| (t.ops, t.pending_ops, t.pending_ops_at)))
        };
        let have = ops_now()?;
        match have {
            None if !send => println!("  set_ops {o}: to do (simulated with --send once init_treasury lands)"),
            Some((h, p, at)) if h != o && h != Pubkey::default() && p == o && at != 0 => {
                println!("  set_ops {o}: proposed, the treasury's ops is still {h}; anyone applies it with apply_ops from unix time {at}")
            }
            _ => {
                let replacing = matches!(have, Some((h, _, _)) if h != o && h != Pubkey::default());
                if let Some((h, _, _)) = have.filter(|_| replacing) {
                    println!("  set_ops: the treasury's ops is {h} today; {o} is PROPOSED and applies after a day (apply_ops, anyone)");
                }
                run.step(
                    &format!("set_ops {o}"),
                    matches!(have, Some((h, _, _)) if h == o),
                    vec![set_ops_ix(&core, &owner, &registry, &treasury, &o)],
                    true,
                    &|| match ops_now()? {
                        Some((h, _, _)) if h == o => Ok(()),
                        Some((_, p, at)) if replacing && p == o && at != 0 => Ok(()),
                        h => Err(format!("the treasury's ops reads {h:?}, not {o}")),
                    },
                )?;
            }
        }
    }
    run.step(
        &format!("init_desk (bond {} in [{}, {}], window {} s)", dp.bond, dp.min_bond, dp.max_bond, dp.window),
        account2(&r, rr2, &desk)?.is_some(),
        vec![ix(core, rook_core::instruction::InitDesk { params: dp }.data(),
            rook_core::accounts::InitDesk { owner, payer: owner, registry, desk, system_program: system_program::ID }.to_account_metas(None))],
        true,
        &|| account2(&r, rr2, &desk)?.map(|_| ()).ok_or("no desk".into()),
    )?;

    for (i, want) in args.iter().enumerate() {
        let id = i as u32;
        let slug = order[i]["slug"].as_str().unwrap_or("");
        // A6-15: the plan row BY SLUG (the config address is the one day 1 created, never discovered by fee_claimer)
        let row = rows.iter().find(|w| w.slug == slug).ok_or(format!("refused: day1-plan.json has no {slug}"))?;
        let k = col_pdas(&core, id);
        if row.id != id || row.listing != k.listing || row.claimer != claimer_pda(&backing, &k.listing) {
            return Err(format!("refused: {slug}: day1-plan.json's listing {} / claimer {} are not listing({id}) / claimer(listing({id}))", row.listing, row.claimer));
        }
        println!("{id:>2} {slug}  listing {}  claimer {}  config {}", k.listing, row.claimer, row.config);

        // list_collection: strictly in order, listing_count == i, every field read back
        let have: Option<rook_core::state::Listing> = get(&r, rr2, &k.listing, &core)?;
        if let Some(l) = &have {
            let d = listing_diff(l, id, want);
            if !d.is_empty() {
                return Err(format!("refused: listing {id} ({slug}) exists with other fields (permanent): {}", d.join("; ")));
            }
        } else if !run.blocked || send {
            let reg: rook_core::state::Registry = get(&r, rr2, &registry, &core)?.ok_or("no registry")?;
            if reg.listing_count != id {
                return Err(format!("refused: Registry.listing_count is {}, listing {slug} must be id {id}: something was listed out of order", reg.listing_count));
            }
        }
        let read_back = || -> Result<(), String> {
            let reg: rook_core::state::Registry = get(&r, rr2, &registry, &core)?.ok_or("no registry")?;
            if reg.listing_count != id + 1 {
                return Err(format!("listing_count {} after listing {id}", reg.listing_count));
            }
            let l: rook_core::state::Listing = get(&r, rr2, &k.listing, &core)?.ok_or("no listing")?;
            let d = listing_diff(&l, id, want);
            if d.is_empty() { Ok(()) } else { Err(d.join("; ")) }
        };
        run.step(
            &format!("list_collection {id} {slug}"),
            have.is_some(),
            vec![ix(core, rook_core::instruction::ListCollection { args: want.clone() }.data(),
                rook_core::accounts::ListCollection { owner, payer: owner, registry, listing: k.listing, system_program: system_program::ID }.to_account_metas(None))],
            false,
            &read_back,
        )?;
        run.step(
            &format!("init_wrapper {id}"),
            account2(&r, rr2, &k.wrapper)?.is_some(),
            vec![ix(core, rook_core::instruction::InitWrapper { meta: wx_meta(&order[i], &custody)? }.data(), rook_core::accounts::InitWrapper {
                owner, payer: owner, registry, listing: k.listing, wrapper: k.wrapper, wx_mint: k.wx_mint, item_set: k.item_set,
                metadata: Pubkey::find_program_address(&[METADATA_SEED, MPL_TOKEN_METADATA_ID.as_ref(), k.wx_mint.as_ref()], &MPL_TOKEN_METADATA_ID).0,
                token_metadata_program: MPL_TOKEN_METADATA_ID, token_program: spl, system_program: system_program::ID }.to_account_metas(None))],
            false,
            &|| account2(&r, rr2, &k.wrapper)?.map(|_| ()).ok_or("no wrapper".into()),
        )?;
        run.step(
            &format!("init_module {id} (revenue_target {} lamports)", params.revenue_target),
            account2(&r, rr2, &k.module)?.is_some(),
            vec![ix(core, rook_core::instruction::InitModule { params }.data(), rook_core::accounts::InitModule {
                owner, payer: owner, registry, listing: k.listing, wrapper: k.wrapper, wx_mint: k.wx_mint, module: k.module,
                module_vault: k.vault, module_wx: k.module_wx, token_program: spl, system_program: system_program::ID }.to_account_metas(None))],
            false,
            &|| account2(&r, rr2, &k.module)?.map(|_| ()).ok_or("no module".into()),
        )?;
        let in_treasury = || -> Result<bool, String> {
            Ok(get::<rook_core::state::Treasury>(&r, rr2, &treasury, &core)?.is_some_and(|t| t.collections.contains(&k.module)))
        };
        run.step(
            &format!("add_collection {id} (treasury)"),
            in_treasury()?,
            vec![ix(core, rook_core::instruction::AddCollection {}.data(),
                rook_core::accounts::AddCollection { owner, registry, treasury, module: k.module }.to_account_metas(None))],
            false,
            &|| if in_treasury()? { Ok(()) } else { Err("module not in the treasury".into()) },
        )?;

        // rook_backing: the partner (every pinned field read back) and the day-1 config of THIS slug
        let partner = Pubkey::find_program_address(&[PARTNER_SEED, k.listing.as_ref()], &backing).0;
        let partner_ok = |p: &rook_backing::state::Partner| -> Result<(), String> {
            let want = (k.listing, registry, k.module, k.vault, k.wrapper, k.wx_mint, row.claimer);
            let have = (p.listing, p.registry, p.module, p.module_vault, p.wrapper, p.wx_mint, p.claimer);
            if have == want { Ok(()) } else { Err(format!("partner {partner} pins {have:?}, want {want:?}")) }
        };
        let p_have: Option<rook_backing::state::Partner> = get(&r, rr2, &partner, &backing)?;
        if let Some(p) = &p_have {
            partner_ok(p).map_err(|e| format!("refused: {slug}: {e}"))?;
        }
        let pp = rook_backing::PartnerParams { listing: k.listing, registry, module: k.module, module_vault: k.vault, wrapper: k.wrapper, wx_mint: k.wx_mint };
        run.step(
            &format!("register_partner {id} {slug}"),
            p_have.is_some(),
            vec![ix(backing, rook_backing::instruction::RegisterPartner { params: pp }.data(),
                rook_backing::accounts::RegisterPartner { admin: owner, settings, registry, partner, listing: k.listing, module: k.module,
                    module_vault: k.vault, wrapper: k.wrapper, system_program: system_program::ID }.to_account_metas(None))],
            false,
            &|| partner_ok(&get(&r, rr2, &partner, &backing)?.ok_or("no partner")?),
        )?;
        // the config: from the plan by slug; it must pay this listing's claimer (preflight checked the whole account)
        let (co, cd) = account2(&r, rr2, &row.config)?.ok_or(format!("refused: {slug}: no config {}", row.config))?;
        check_written(&co, &cd, &row.claimer, row.threshold).map_err(|e| format!("refused: {slug}: {e}"))?;
        let registered = || -> Result<bool, String> {
            Ok(get::<rook_backing::state::Partner>(&r, rr2, &partner, &backing)?.is_some_and(|p| p.configs.contains(&row.config)))
        };
        run.step(
            &format!("register_partner_config {id} {slug} ({})", row.config),
            registered()?,
            vec![ix(backing, rook_backing::instruction::RegisterPartnerConfig {}.data(),
                rook_backing::accounts::RegisterPartnerConfig { admin: owner, settings, registry, partner, dbc_config: row.config }.to_account_metas(None))],
            false,
            &|| if registered()? { Ok(()) } else { Err("the config is not in the partner".into()) },
        )?;
    }
    println!("{}", if send {
        "listing ceremony: done, every account read back. Next, at once: rook-ceremony propose-owner, then the fund wallet's accept-owner, then verify-owner"
    } else {
        "(nothing sent: --owner <keys>/ceremony-payer.json --send)"
    });
    Ok(())
}

/// The canister's Solana address: --canister-sol and/or the launch file's `canister.sol_address` (agreeing), equal to every custodian
/// the custody file names (it is the canister's Ed25519 key too).
pub fn canister_sol_of(a: &BTreeMap<String, String>, launch: &Value, custody: &Value) -> Result<Pubkey, String> {
    let flag = a.get("canister-sol").cloned();
    let file = launch.pointer("/canister/sol_address").and_then(|v| v.as_str()).map(String::from);
    let s = match (flag, file) {
        (Some(x), Some(y)) if x != y => return Err(format!("refused: --canister-sol {x} is not the launch file's canister.sol_address {y}")),
        (Some(x), _) | (None, Some(x)) => x,
        (None, None) => return Err("--canister-sol <the canister's Solana address> is required (or the launch file's canister.sol_address): the canister is the registry keeper (D-i)".into()),
    };
    let k: Pubkey = s.parse().map_err(|_| format!("--canister-sol {s}: not a base58 Solana address"))?;
    let mut custodians: Vec<&str> = custody.get("custodian").and_then(|v| v.as_str()).into_iter().collect();
    if let Some(m) = custody.get("per_slug").and_then(|v| v.as_object()) {
        custodians.extend(m.values().filter_map(|v| v.get("custodian").and_then(|c| c.as_str())));
    }
    if let Some(c) = custodians.iter().find(|c| **c != s) {
        return Err(format!("refused: the canister's Solana address {s} is not custody.json's custodian {c}"));
    }
    Ok(k)
}

/// Whether the keeper is already the canister (true), unset (false), or refused: another keeper (a 7-day move), or a guardian / owner
/// equal to the keeper.
pub fn check_keeper(reg: &rook_core::state::Registry, canister: &Pubkey, fund: &Pubkey) -> Result<bool, String> {
    if *canister == reg.owner || *canister == reg.guardian || *canister == *fund {
        return Err(format!("refused: the canister's key {canister} is the registry's owner, its guardian or the fund wallet: the keeper must differ"));
    }
    if reg.guardian != Pubkey::default() && reg.guardian == *fund {
        return Err(format!("refused: the registry's guardian is the fund wallet {fund}, which must become the owner (propose_owner refuses the guardian)"));
    }
    if reg.guardian != Pubkey::default() && reg.guardian == reg.owner {
        return Err(format!("refused: the registry's guardian {} is its owner: the guardian must differ from the owner and the keeper", reg.guardian));
    }
    if reg.keeper == Pubkey::default() {
        return Ok(false);
    }
    if reg.keeper != *canister {
        return Err(format!("refused: Registry.keeper is already {} (not the canister {canister}); moving it is a 7-day change (propose_roles, then accept_roles)", reg.keeper));
    }
    if reg.keeper_paused {
        return Err(format!("refused: the keeper {canister} is set but paused: the owner unpauses it (unpause_keeper) after checking why"));
    }
    Ok(true)
}

/// `propose_roles(keeper)` (rook_core, owner-signed: RegistryAdmin { signer, registry }).
pub fn propose_roles_ix(core: &Pubkey, owner: &Pubkey, registry: &Pubkey, keeper: &Pubkey) -> Instruction {
    Instruction {
        program_id: *core,
        accounts: rook_core::accounts::RegistryAdmin { signer: *owner, registry: *registry }.to_account_metas(None),
        data: rook_core::instruction::ProposeRoles { keeper: *keeper }.data(),
    }
}

/// `set_ops(ops)` (rook_core, owner-signed: TreasuryAdmin { owner, registry, treasury }).
pub fn set_ops_ix(core: &Pubkey, owner: &Pubkey, registry: &Pubkey, treasury: &Pubkey, ops: &Pubkey) -> Instruction {
    Instruction {
        program_id: *core,
        accounts: rook_core::accounts::TreasuryAdmin { owner: *owner, registry: *registry, treasury: *treasury }.to_account_metas(None),
        data: rook_core::instruction::SetOps { ops: *ops }.data(),
    }
}

/// --ops must be a plain key: not the default, not one of the programs or their treasury / registry PDAs.
pub fn check_ops(ops: &Pubkey, core: &Pubkey, backing: &Pubkey, treasury: &Pubkey, registry: &Pubkey) -> Result<(), String> {
    if *ops == Pubkey::default() || [core, backing, treasury, registry].contains(&ops) {
        return Err(format!("refused: --ops {ops} is the default key, a program or a rook_core PDA, not the fueler's hot key"));
    }
    Ok(())
}

/// Account sizes of the listing ceremony (data bytes; rent adds the 128-byte overhead), for ops/solana/fund-status.mjs (audit A6-11).
/// The Metaplex metadata account is taken at its maximum (679 bytes, MAX_METADATA_LEN).
#[cfg_attr(not(test), allow(dead_code))]
pub fn listing_ceremony_sizes() -> Value {
    use anchor_lang::Space;
    use rook_core::state::*;
    json!({
        "once": [8 + Treasury::INIT_SPACE, 8 + Desk::INIT_SPACE],
        "per_listing": [8 + Listing::INIT_SPACE, 8 + Wrapper::INIT_SPACE, 82, 8 + ItemSet::INIT_SPACE, 679, 8 + Module::INIT_SPACE,
            8 + ModuleVault::INIT_SPACE, 165, 8 + rook_backing::state::Partner::INIT_SPACE],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// fund-status.mjs's LISTING_CEREMONY sizes are the programs' own (rerun after any account change).
    #[test]
    fn fund_status_listing_ceremony_sizes_are_the_programs() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../..");
        let src = fs::read_to_string(root.join("ops/solana/fund-status.mjs")).unwrap();
        let line = src.lines().find(|l| l.starts_with("export const LISTING_CEREMONY_SIZES = ")).expect("LISTING_CEREMONY_SIZES in fund-status.mjs");
        let j: Value = serde_json::from_str(line.trim_start_matches("export const LISTING_CEREMONY_SIZES = ").trim_end_matches(';')).unwrap();
        assert_eq!(j, listing_ceremony_sizes(), "update fund-status.mjs LISTING_CEREMONY_SIZES to {}", listing_ceremony_sizes());
    }

    fn entry(kind: &str) -> Value {
        if kind == "eth" {
            json!({ "listing_id": 0, "slug": "milady", "kind": "EthNft", "name": "Wrapped Milady", "symbol": "wMILADY", "chain_id": 1,
                "collection": "0x5af0d9827e0c53e4799bb226655a1de152a425a5" })
        } else {
            json!({ "listing_id": 11, "slug": "bitcoin-frogs", "kind": "Inscriptions", "name": "Wrapped Bitcoin Frog", "symbol": "wBFROG",
                "members_root": "0x801f71bbb4c7ee37759a3b23da76bc3dd5ec132c0dd10edefe3dc86acfae418e" })
        }
    }

    /// ListArgs come from listing_order + the custody file only; every field read back is compared.
    #[test]
    fn list_args_and_listing_diff() {
        let (c, p) = (Pubkey::new_unique(), Pubkey::new_unique());
        let custody = json!({ "custodian": c.to_string(), "poster": p.to_string(), "eth_custody": "0x00000000000000000000000000000000000000aa",
            "btc_custody": "bc1pxxxxxxxx", "per_slug": { "bitcoin-frogs": { "btc_custody": "bc1pfrogs" } } });
        let e = list_args(&entry("eth"), &custody).unwrap();
        assert_eq!((e.name.as_str(), e.custody.as_str(), e.custodian, e.poster), ("Wrapped Milady", "0x00000000000000000000000000000000000000aa", c, p));
        assert!(matches!(e.kind, ListingKind::EthNft { chain_id: 1, .. }));
        let f = list_args(&entry("frog"), &custody).unwrap();
        assert_eq!((f.custody.as_str(), f.poster), ("bc1pfrogs", Pubkey::default()));
        assert!(matches!(f.kind, ListingKind::Inscriptions { membership_root } if membership_root[0] == 0x80));
        assert!(list_args(&entry("eth"), &json!({ "custodian": c.to_string() })).unwrap_err().contains("eth_custody"));
        assert_eq!(wx_meta(&entry("eth"), &custody).unwrap().symbol, "wMILADY");

        let mut l = rook_core::state::Listing {
            id: 0, kind: e.kind, paused: false, custodian: c, poster: p, poster_paused: false, poster_resumed_at: 0, custody: e.custody.clone(),
            custody_evm: [0; 20], name: e.name.clone(), pending_root: [0; 32], pending_root_at: 0, bump: 255,
        };
        assert!(listing_diff(&l, 0, &e).is_empty());
        assert_eq!(listing_diff(&l, 1, &e).len(), 1, "another id");
        l.custodian = Pubkey::new_unique();
        l.name = "Wrapped Azuki".into();
        assert_eq!(listing_diff(&l, 0, &e).len(), 2);
    }

    /// --ops: set_ops is rook_core's own instruction (owner signer, registry, treasury mut), and the obvious wrong keys are refused.
    #[test]
    fn set_ops_instruction_and_refusals() {
        use anchor_lang::Discriminator;
        let (core, backing, fund, hot) = (rook_core::ID, Pubkey::new_unique(), Pubkey::new_unique(), Pubkey::new_unique());
        let registry = Pubkey::find_program_address(&[b"registry"], &core).0;
        let treasury = Pubkey::find_program_address(&[b"treasury"], &core).0;
        let ix = set_ops_ix(&core, &fund, &registry, &treasury, &hot);
        assert_eq!(ix.program_id, core);
        assert_eq!(&ix.data[..8], rook_core::instruction::SetOps::DISCRIMINATOR);
        assert_eq!(&ix.data[8..], hot.as_ref());
        let m: Vec<_> = ix.accounts.iter().map(|a| (a.pubkey, a.is_signer, a.is_writable)).collect();
        assert_eq!(m, vec![(fund, true, false), (registry, false, false), (treasury, false, true)]);
        assert!(check_ops(&hot, &core, &backing, &treasury, &registry).is_ok());
        for bad in [Pubkey::default(), core, backing, treasury, registry] {
            assert!(check_ops(&bad, &core, &backing, &treasury, &registry).is_err());
        }
    }

    /// D-i: the keeper is the canister: from the flag or the launch file (agreeing), equal to the custodians; propose_roles is rook_core's.
    #[test]
    fn keeper_is_the_canister() {
        use anchor_lang::Discriminator;
        let (k, other) = (Pubkey::new_unique(), Pubkey::new_unique());
        let flag = |v: &str| BTreeMap::from([("canister-sol".to_string(), v.to_string())]);
        let none = BTreeMap::new();
        let custody = json!({ "custodian": k.to_string(), "per_slug": { "x": { "custodian": k.to_string() } } });
        assert_eq!(canister_sol_of(&flag(&k.to_string()), &json!({}), &custody).unwrap(), k);
        assert_eq!(canister_sol_of(&none, &json!({ "canister": { "sol_address": k.to_string() } }), &json!({})).unwrap(), k);
        assert!(canister_sol_of(&none, &json!({}), &custody).unwrap_err().contains("required"));
        assert!(canister_sol_of(&flag(&k.to_string()), &json!({ "canister": { "sol_address": other.to_string() } }), &custody).is_err());
        assert!(canister_sol_of(&flag(&other.to_string()), &json!({}), &custody).unwrap_err().contains("custodian"));
        let ix = propose_roles_ix(&rook_core::ID, &other, &k, &k);
        assert_eq!(&ix.data[..8], rook_core::instruction::ProposeRoles::DISCRIMINATOR);
        assert_eq!(&ix.data[8..], k.as_ref());
        assert_eq!(ix.accounts.iter().map(|a| (a.is_signer, a.is_writable)).collect::<Vec<_>>(), vec![(true, false), (false, true)]);
    }

    /// The listing-ceremony PDAs are rook_core's own (with the dev ids compiled in).
    #[test]
    fn col_pdas_are_the_programs() {
        let k = col_pdas(&rook_core::ID, 3);
        assert_eq!(k.listing, listing_pda(&rook_core::ID, 3));
        assert_eq!(k.wrapper, Pubkey::find_program_address(&[WRAPPER, k.listing.as_ref()], &rook_core::ID).0);
        assert_eq!(k.vault, Pubkey::find_program_address(&[MODULE_VAULT, k.module.as_ref()], &rook_core::ID).0);
    }
}
