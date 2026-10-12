//! The registry ownership handover at the end of the setup: the fund wallet (the operator's Phantom / Ledger wallet,
//! GGgXnNgt…6Wuy) never exists as a key file. `init` names the ceremony key (<keys>/ceremony-payer.json, generated on day 1) as the
//! registry owner, the listing ceremony signs with it, and then:
//!
//!   rook-ceremony propose-owner --plan <keys>/day1/day1-plan.json --rpc <url> [--rpc2 <url>] --owner <keys>/ceremony-payer.json
//!                               [--send [--yes]] [--allow-incomplete]
//!       rook_core `propose_owner(fund wallet)` signed by the ceremony key, read back (Registry.pending_owner = the fund wallet).
//!       Refused before the listing ceremony is complete (treasury, desk, every listing of day1-plan.json; --allow-incomplete).
//!   rook-ceremony accept-owner --plan <…> --rpc <url> --print-tx [--page-rpc <url>]
//!       the UNSIGNED `accept_owner` transaction (fee payer and only signer: the fund wallet), simulated, printed in base64 and base58,
//!       with the link to ops/solana/accept-owner.html, which has Phantom (or a Ledger inside Phantom) sign and send it.
//!   rook-ceremony accept-owner --plan <…> --rpc <url> --ledger usb://ledger[?key=…] [--send [--yes]]
//!       the same transaction signed on a Ledger (the solana CLI's locator syntax), sent, read back.
//!       (--keypair <file>: a stand-in fund wallet for a local rehearsal only; refused on mainnet-beta's genesis.)
//!   rook-ceremony verify-owner --plan <…> --rpc <url> [--rpc2 <url>]
//!       Registry.owner = the fund wallet, no pending owner, rook_backing's admin (it reads Registry.owner) follows, both upgrade
//!       authorities = the fund wallet, and the ceremony key holds no role (keeper, guardian, treasury ops). Exit 1 otherwise.
//!
//! Until `accept_owner` lands the ceremony key IS the owner: accept at once, then delete the ceremony key file.
use super::*;
use anchor_lang::AccountDeserialize;
use rook_core::state::Registry;

pub struct Ctx {
    pub r: Rpc,
    pub r2: Option<Rpc>,
    pub core: Pubkey,
    pub backing: Pubkey,
    pub fund: Pubkey,
    pub setup: Pubkey,
    pub registry: Pubkey,
    pub mainnet: bool,
    pub listings: usize,
}

/// The plan names the ids, the fund wallet and the ceremony key; the RPC must serve the plan's cluster (and genesis).
fn ctx(a: &BTreeMap<String, String>) -> Result<Ctx, String> {
    let need = |k: &str| a.get(k).cloned().ok_or(format!("--{k} is required"));
    let plan = read_json(&need("plan").map_err(|_| "--plan <keys>/day1/day1-plan.json is required")?)?;
    let fund: Pubkey = a.get("fund-wallet").map(String::as_str).unwrap_or(FUND_WALLET).parse().map_err(|_| "--fund-wallet")?;
    if pk_of(&plan["fund_wallet"], "fund_wallet")? != fund {
        return Err(format!("refused: the fund wallet {fund} is not day1-plan.json's {}", plan["fund_wallet"]));
    }
    let r = Rpc { url: need("rpc")? };
    let r2 = a.get("rpc2").map(|u| Rpc { url: u.clone() });
    let g = r.call("getGenesisHash", json!([]))?.as_str().unwrap_or("").to_string();
    if let Some(r2) = &r2 {
        if r2.call("getGenesisHash", json!([]))?.as_str() != Some(&g) {
            return Err("refused: the two RPCs serve different clusters".into());
        }
    }
    check_plan_cluster(&plan, &g)?;
    let core = pk_of(&plan["rook_core"], "rook_core")?;
    Ok(Ctx {
        r,
        r2,
        core,
        backing: pk_of(&plan["rook_backing"], "rook_backing")?,
        fund,
        setup: pk_of(&plan["ceremony_payer"], "ceremony_payer")?,
        registry: Pubkey::find_program_address(&[b"registry"], &core).0,
        mainnet: g == MAINNET_GENESIS,
        listings: plan["listings"].as_array().map(|v| v.len()).unwrap_or(0),
    })
}

pub fn check_plan_cluster(plan: &Value, genesis: &str) -> Result<(), String> {
    if let Some(pg) = plan["genesis"].as_str() {
        if pg != genesis {
            return Err(format!("refused: day 1 ran on genesis {pg}; this RPC is {genesis}"));
        }
    }
    match plan["cluster"].as_str().unwrap_or("") {
        "mainnet-beta" if genesis != MAINNET_GENESIS => Err(format!("refused: day1-plan.json is mainnet-beta's, the RPC's genesis is {genesis}")),
        "devnet" if genesis != DEVNET_GENESIS => Err(format!("refused: day1-plan.json is devnet's, the RPC's genesis is {genesis}")),
        "localnet" if genesis == MAINNET_GENESIS || genesis == DEVNET_GENESIS => Err(format!("refused: day1-plan.json is localnet's, the RPC is a public cluster ({genesis})")),
        "mainnet-beta" | "devnet" | "localnet" => Ok(()),
        c => Err(format!("refused: day1-plan.json's cluster {c:?}")),
    }
}

fn registry(c: &Ctx) -> Result<Registry, String> {
    let (o, d) = account2(&c.r, c.r2.as_ref(), &c.registry)?.ok_or(format!("refused: no registry {} (deploy-later.sh runs init)", c.registry))?;
    if o != c.core {
        return Err(format!("refused: {} is owned by {o}, not rook_core {}", c.registry, c.core));
    }
    Registry::try_deserialize(&mut &d[..]).map_err(|e| format!("not a rook_core Registry: {e}"))
}

pub fn propose_owner_ix(core: &Pubkey, owner: &Pubkey, registry: &Pubkey, new_owner: &Pubkey) -> Instruction {
    Instruction {
        program_id: *core,
        accounts: rook_core::accounts::RegistryAdmin { signer: *owner, registry: *registry }.to_account_metas(None),
        data: rook_core::instruction::ProposeOwner { new_owner: *new_owner }.data(),
    }
}

/// `accept_owner`: RegistryAdmin { signer = the pending owner (the fund wallet), registry (writable) }. The fund wallet is also the fee
/// payer, so the transaction carries exactly one signature.
pub fn accept_owner_ix(core: &Pubkey, fund: &Pubkey, registry: &Pubkey) -> Instruction {
    Instruction {
        program_id: *core,
        accounts: rook_core::accounts::RegistryAdmin { signer: *fund, registry: *registry }.to_account_metas(None),
        data: rook_core::instruction::AcceptOwner {}.data(),
    }
}

fn simulate_ix(r: &Rpc, ix: &Instruction, payer: &Pubkey) -> Result<u64, String> {
    let msg = message(std::slice::from_ref(ix), payer, solana_message::Hash::default());
    let tx = VersionedTransaction { signatures: vec![Default::default(); msg.header().num_required_signatures as usize], message: msg };
    let sim = r.call("simulateTransaction", json!([b64(&tx), { "encoding": "base64", "sigVerify": false, "replaceRecentBlockhash": true, "commitment": "confirmed" }]))?;
    if !sim["value"]["err"].is_null() {
        return Err(format!("refused in simulation: {} {}", sim["value"]["err"], sim["value"]["logs"]));
    }
    Ok(sim["value"]["unitsConsumed"].as_u64().unwrap_or(0))
}

fn confirm(a: &BTreeMap<String, String>, question: &str, word: &str) -> Result<(), String> {
    if a.contains_key("yes") {
        return Ok(());
    }
    print!("{question} Type '{word}': ");
    std::io::stdout().flush().ok();
    let mut s = String::new();
    std::io::stdin().read_line(&mut s).map_err(|e| e.to_string())?;
    if s.trim() != word {
        return Err("not sent".into());
    }
    Ok(())
}

/// `rook-ceremony propose-owner`: the ceremony key proposes the fund wallet (Ownable2Step's first step).
pub fn propose_owner(a: &BTreeMap<String, String>) -> Result<(), String> {
    let c = ctx(a)?;
    let send = a.contains_key("send");
    let kp = a.get("owner").map(|p| read_kp(Path::new(p))).transpose()?;
    if send && kp.is_none() {
        return Err("refused: --send needs --owner <keys>/ceremony-payer.json (the registry owner during the setup)".into());
    }
    if let Some(k) = &kp {
        if k.pubkey() != c.setup {
            return Err(format!("refused: --owner is {}, not the ceremony key {} (day1-plan.json ceremony_payer)", k.pubkey(), c.setup));
        }
    }
    let reg = registry(&c)?;
    if reg.owner == c.fund {
        println!("propose-owner: the registry already belongs to the fund wallet {} (handed over); run verify-owner", c.fund);
        return Ok(());
    }
    if reg.owner != c.setup {
        return Err(format!("refused: the registry's owner is {}, neither the ceremony key {} nor the fund wallet {}", reg.owner, c.setup, c.fund));
    }
    if reg.guardian == c.fund {
        return Err(format!("refused: the registry's guardian is the fund wallet {}: propose_owner refuses the guardian (set_guardian first)", c.fund));
    }
    // the ceremony must be complete: after the handover a missing step would need the fund wallet's signature (never a key file)
    let treasury = Pubkey::find_program_address(&[b"treasury"], &c.core).0;
    let desk = Pubkey::find_program_address(&[b"desk"], &c.core).0;
    let mut missing = vec![];
    if (reg.listing_count as usize) < c.listings {
        missing.push(format!("{} of {} listings", reg.listing_count, c.listings));
    }
    for (name, k) in [("treasury", treasury), ("desk", desk)] {
        if account2(&c.r, c.r2.as_ref(), &k)?.is_none() {
            missing.push(format!("no {name}"));
        }
    }
    if reg.keeper == Pubkey::default() {
        missing.push("no keeper (propose_roles)".into());
    }
    if !missing.is_empty() {
        if !a.contains_key("allow-incomplete") {
            return Err(format!("refused: the listing ceremony is not complete ({}): finish `rook-ceremony list --send` first, while the \
                ceremony key can still sign (or --allow-incomplete)", missing.join(", ")));
        }
        println!("!! --allow-incomplete: the listing ceremony is not complete ({})", missing.join(", "));
    }
    if reg.pending_owner == c.fund {
        println!("propose-owner: already proposed (Registry.pending_owner = the fund wallet {}); next: accept-owner", c.fund);
        return Ok(());
    }
    if reg.pending_owner != Pubkey::default() {
        println!("propose-owner: the pending owner {} is replaced by the fund wallet", reg.pending_owner);
    }
    let ix = propose_owner_ix(&c.core, &c.setup, &c.registry, &c.fund);
    let cu = simulate_ix(&c.r, &ix, &c.setup).map_err(|e| format!("propose_owner: {e}"))?;
    print!("propose_owner({}) signed by the ceremony key {}: simulated OK ({cu} CU)", c.fund, c.setup);
    if !send {
        println!(", not sent (--owner <keys>/ceremony-payer.json --send)");
        return Ok(());
    }
    println!();
    let fs = c.fund.to_string();
    confirm(a, &format!("Propose the fund wallet {fs} (last 6 characters: {}) as the registry owner?", &fs[fs.len() - 6..]), "propose")?;
    let sig = send_one(&c.r, &[ix], &[kp.as_ref().unwrap()])?;
    let reg = registry(&c)?;
    if reg.pending_owner != c.fund || reg.owner != c.setup {
        return Err(format!("propose_owner sent {sig}, but the read-back FAILED: owner {}, pending owner {}", reg.owner, reg.pending_owner));
    }
    println!("propose_owner: SENT {sig}; read back: owner {} (the ceremony key), pending owner {} (the fund wallet)", reg.owner, reg.pending_owner);
    println!("NOW, without delay: the fund wallet signs accept_owner (rook-ceremony accept-owner --print-tx, or --ledger usb://ledger)");
    Ok(())
}

pub fn b58(bytes: &[u8]) -> String {
    const A: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut digits: Vec<u8> = vec![];
    for &b in bytes {
        let mut carry = b as u32;
        for d in digits.iter_mut() {
            carry += (*d as u32) << 8;
            *d = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let zeros = bytes.iter().take_while(|b| **b == 0).count();
    std::iter::repeat_n('1', zeros).chain(digits.iter().rev().map(|d| A[*d as usize] as char)).collect()
}

fn url_component(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

fn ledger_signer(uri: &str) -> Result<Box<dyn Signer>, String> {
    use solana_remote_wallet::{locator::Locator, remote_keypair::generate_remote_keypair, remote_wallet::maybe_wallet_manager};
    let u = uriparse::URIReference::try_from(uri).map_err(|e| format!("--ledger {uri}: {e}"))?;
    let path = solana_derivation_path::DerivationPath::from_uri_key_query(&u).map_err(|e| format!("--ledger {uri}: {e}"))?.unwrap_or_default();
    let loc = Locator::new_from_uri(&u).map_err(|e| format!("--ledger {uri}: {e}"))?;
    let wm = maybe_wallet_manager()
        .map_err(|e| format!("hardware wallet: {e}"))?
        .ok_or("no hardware wallet found: plug in the Ledger, unlock it and open the Solana app")?;
    let k = generate_remote_keypair(loc, path, &wm, false, "fund wallet").map_err(|e| format!("Ledger: {e}"))?;
    Ok(Box::new(k))
}

/// Sends one transaction already signed (the same bytes until the blockhash expires: at most one lands), until finalized.
fn send_signed(r: &Rpc, tx: &VersionedTransaction, last_valid: u64) -> Result<String, String> {
    let sig = tx.signatures[0].to_string();
    let wire = b64(tx);
    loop {
        let _ = r.call("sendTransaction", json!([wire, { "encoding": "base64", "skipPreflight": false, "preflightCommitment": "confirmed" }]));
        sleep(Duration::from_secs(3));
        let s = r.call("getSignatureStatuses", json!([[sig], { "searchTransactionHistory": true }]))?;
        let st = &s["value"][0];
        if !st.is_null() {
            if !st["err"].is_null() {
                return Err(format!("{sig} failed: {}", st["err"]));
            }
            if st["confirmationStatus"] == json!("finalized") {
                return Ok(sig);
            }
        }
        let h = r.call("getBlockHeight", json!([{ "commitment": "finalized" }]))?.as_u64().unwrap_or(0);
        if h > last_valid && st.is_null() {
            return Err(format!("{sig} expired unlanded: rerun accept-owner"));
        }
    }
}

/// `rook-ceremony accept-owner`: the fund wallet's one signature, as an unsigned transaction for Phantom (--print-tx) or on a Ledger.
pub fn accept_owner(a: &BTreeMap<String, String>) -> Result<(), String> {
    let c = ctx(a)?;
    let reg = registry(&c)?;
    if reg.owner == c.fund && reg.pending_owner == Pubkey::default() {
        println!("accept-owner: already done (Registry.owner = the fund wallet {}, nothing pending); run verify-owner", c.fund);
        return Ok(());
    }
    if reg.pending_owner != c.fund {
        return Err(format!("refused: the registry's pending owner is {}, not the fund wallet {}: run propose-owner first",
            if reg.pending_owner == Pubkey::default() { "unset".to_string() } else { reg.pending_owner.to_string() }, c.fund));
    }
    let bal = c.r.call("getBalance", json!([c.fund.to_string(), { "commitment": "confirmed" }]))?["value"].as_u64().unwrap_or(0);
    if bal < LAMPORTS_PER_SIGNATURE {
        return Err(format!("refused: the fund wallet {} holds {bal} lamports; it pays the 5,000-lamport fee of accept_owner", c.fund));
    }
    let ix = accept_owner_ix(&c.core, &c.fund, &c.registry);
    let cu = simulate_ix(&c.r, &ix, &c.fund).map_err(|e| format!("accept_owner: {e}"))?;
    println!("accept_owner (rook_core {}, registry {}), fee payer and only signer the fund wallet {}: simulated OK ({cu} CU)", c.core, c.registry, c.fund);
    let v = c.r.call("getLatestBlockhash", json!([{ "commitment": "finalized" }]))?;
    let bh: solana_message::Hash = v["value"]["blockhash"].as_str().unwrap_or("").parse().map_err(|_| "blockhash")?;
    let last = v["value"]["lastValidBlockHeight"].as_u64().unwrap_or(0);
    let msg = message(&[ix], &c.fund, bh);
    if a.contains_key("print-tx") {
        let tx = VersionedTransaction { signatures: vec![Default::default()], message: msg };
        let bytes = bincode_tx(&tx);
        let page_rpc = a.get("page-rpc").cloned().unwrap_or_else(|| if c.mainnet { "https://api.mainnet-beta.solana.com".into() } else { c.r.url.clone() });
        let b64s = base64::engine::general_purpose::STANDARD.encode(&bytes);
        println!("\nThe UNSIGNED transaction ({} bytes; one signature slot, the fund wallet's; blockhash {bh}, valid ≈ 60 s: the page below", bytes.len());
        println!("replaces it with a fresh one before asking the wallet, which is safe because nothing is signed yet):");
        println!("  base64: {b64s}");
        println!("  base58: {}", b58(&bytes));
        println!("\nSign it with Phantom (or a Ledger connected to Phantom), from this repository:");
        println!("  python3 -m http.server 8765 --bind 127.0.0.1 --directory ops/solana");
        println!("  open \"http://127.0.0.1:8765/accept-owner.html?tx={}&rpc={}&program={}&owner={}\"",
            url_component(&b64s), url_component(&page_rpc), c.core, c.fund);
        println!("The page decodes the transaction and shows: program {}, instruction accept_owner, registry {}, signer {}.", c.core, c.registry, c.fund);
        println!("Check them, connect the fund wallet, press Sign and send. Then: rook-ceremony verify-owner");
        return Ok(());
    }
    let signer: Box<dyn Signer> = match (a.get("ledger"), a.get("keypair")) {
        (Some(u), None) => ledger_signer(u)?,
        (None, Some(p)) => {
            if c.mainnet {
                return Err("refused: --keypair on mainnet-beta: the fund wallet is never a key file (--print-tx for Phantom, or --ledger)".into());
            }
            Box::new(read_kp(Path::new(p))?)
        }
        _ => return Err("accept-owner needs one of: --print-tx (Phantom), --ledger usb://ledger[?key=…], --keypair <file> (local rehearsal)".into()),
    };
    let who = signer.try_pubkey().map_err(|e| format!("signer: {e}"))?;
    if who != c.fund {
        return Err(format!("refused: the signer is {who}, not the fund wallet {} (the pending owner){}", c.fund,
            if a.contains_key("ledger") { "; try another account: usb://ledger?key=0, ?key=0/0, ?key=1 … (solana-keygen pubkey <uri>)" } else { "" }));
    }
    if !a.contains_key("send") {
        println!("(not sent: add --send; the {} signs then)", if a.contains_key("ledger") { "Ledger" } else { "keypair" });
        return Ok(());
    }
    let fs = c.fund.to_string();
    confirm(a, &format!("Sign accept_owner as the fund wallet {fs} (last 6: {})?", &fs[fs.len() - 6..]), "accept")?;
    if a.contains_key("ledger") {
        println!("confirm on the Ledger (the Solana app may need \"Blind signing\" enabled for rook_core's instruction)");
    }
    let sig = signer.try_sign_message(&msg.serialize()).map_err(|e| format!("signing: {e}"))?;
    let tx = VersionedTransaction { signatures: vec![sig], message: msg };
    let sig = send_signed(&c.r, &tx, last)?;
    let reg = registry(&c)?;
    if reg.owner != c.fund || reg.pending_owner != Pubkey::default() {
        return Err(format!("accept_owner sent {sig}, but the read-back FAILED: owner {}, pending {}", reg.owner, reg.pending_owner));
    }
    println!("accept_owner: SENT {sig}; read back: Registry.owner = the fund wallet {}, nothing pending. Next: verify-owner", c.fund);
    Ok(())
}

/// `rook-ceremony verify-owner`: the state to announce, read back (through both RPCs with --rpc2).
pub fn verify_owner(a: &BTreeMap<String, String>) -> Result<(), String> {
    use anchor_lang::solana_program::bpf_loader_upgradeable;
    let c = ctx(a)?;
    let reg = registry(&c)?;
    let mut fail = vec![];
    let mut line = |ok: bool, what: String| {
        println!("  {} {what}", if ok { "ok  " } else { "FAIL" });
        if !ok {
            fail.push(what);
        }
    };
    println!("verify-owner: rook_core {}, registry {}", c.core, c.registry);
    line(reg.owner == c.fund, format!("Registry.owner = {} (want the fund wallet {})", reg.owner, c.fund));
    line(reg.pending_owner == Pubkey::default(), format!("Registry.pending_owner = {} (want none{})", reg.pending_owner,
        if reg.pending_owner == c.fund { ": the fund wallet has not signed accept_owner yet" } else { "" }));
    let settings = Pubkey::find_program_address(&[rook_backing::constants::SETTINGS_SEED], &c.backing).0;
    match account2(&c.r, c.r2.as_ref(), &settings)? {
        Some((o, d)) if o == c.backing => {
            let s = rook_backing::state::Settings::try_deserialize(&mut &d[..]).map_err(|e| format!("rook_backing settings: {e}"))?;
            line(s.registry == c.registry, format!("rook_backing's admin is Registry.owner of {} (its settings pin {})", c.registry, s.registry));
        }
        _ => line(false, format!("rook_backing settings {settings} exist")),
    }
    for (name, p) in [("rook_core", c.core), ("rook_backing", c.backing)] {
        let pd = Pubkey::find_program_address(&[p.as_ref()], &bpf_loader_upgradeable::ID).0;
        let ua = account2(&c.r, c.r2.as_ref(), &pd)?.and_then(|(_, d)| rook_core::upgrade_authority(&d));
        line(ua == Some(c.fund), format!("{name} upgrade authority = {} (want the fund wallet)", ua.map(|k| k.to_string()).unwrap_or("none".into())));
    }
    let treasury = Pubkey::find_program_address(&[b"treasury"], &c.core).0;
    let t: Option<rook_core::state::Treasury> = match account2(&c.r, c.r2.as_ref(), &treasury)? {
        Some((o, d)) if o == c.core => Some(rook_core::state::Treasury::try_deserialize(&mut &d[..]).map_err(|e| format!("treasury: {e}"))?),
        _ => None,
    };
    let roles = [
        ("keeper", reg.keeper),
        ("pending keeper", reg.pending_keeper),
        ("guardian", reg.guardian),
        ("treasury ops", t.as_ref().map(|t| t.ops).unwrap_or_default()),
        ("pending treasury ops", t.as_ref().map(|t| t.pending_ops).unwrap_or_default()),
    ];
    let mut held: Vec<&str> = roles.iter().filter(|(_, k)| *k == c.setup).map(|(n, _)| *n).collect();
    if reg.owner == c.setup {
        held.insert(0, "registry owner");
    }
    line(held.is_empty(), format!("the ceremony key {} holds no role{}", c.setup,
        if held.is_empty() { String::new() } else { format!(" (it is the {})", held.join(", ")) }));
    println!("  info keeper {} (paused {}), guardian {}, treasury ops {}", reg.keeper, reg.keeper_paused, reg.guardian,
        t.as_ref().map(|t| t.ops.to_string()).unwrap_or("no treasury".into()));
    if !fail.is_empty() {
        return Err(format!("verify-owner: {} check(s) failed: do not announce", fail.len()));
    }
    println!("verify-owner: OK. The fund wallet owns the registry (and so rook_backing) and both programs. The ceremony key is powerless:");
    println!("  return its SOL (solana transfer --from <keys>/ceremony-payer.json {} ALL), then delete the file (rm -P on macOS).", c.fund);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anchor_lang::Discriminator;

    /// accept_owner is rook_core's own instruction with the fund wallet as the only signer; propose_owner names it.
    #[test]
    fn handover_instructions_are_rook_cores() {
        let (fund, setup) = (Pubkey::new_unique(), Pubkey::new_unique());
        let registry = Pubkey::find_program_address(&[b"registry"], &rook_core::ID).0;
        let ix = accept_owner_ix(&rook_core::ID, &fund, &registry);
        assert_eq!(ix.data, rook_core::instruction::AcceptOwner::DISCRIMINATOR);
        assert_eq!(ix.accounts.iter().map(|m| (m.pubkey, m.is_signer, m.is_writable)).collect::<Vec<_>>(), vec![(fund, true, false), (registry, false, true)]);
        let msg = message(&[ix], &fund, solana_message::Hash::default());
        assert_eq!(msg.header().num_required_signatures, 1, "one signature: the fund wallet's");
        let p = propose_owner_ix(&rook_core::ID, &setup, &registry, &fund);
        assert_eq!(&p.data[..8], rook_core::instruction::ProposeOwner::DISCRIMINATOR);
        assert_eq!(&p.data[8..], fund.as_ref());
        assert_eq!(p.accounts[0].pubkey, setup);
    }

    #[test]
    fn base58_matches_pubkey_display() {
        for _ in 0..20 {
            let k = Pubkey::new_unique();
            assert_eq!(b58(k.as_ref()), k.to_string());
        }
        assert_eq!(b58(&[0, 0, 1]), "112");
        assert_eq!(url_component("a+b/c="), "a%2Bb%2Fc%3D");
    }

    #[test]
    fn the_plan_cluster_is_checked() {
        let p = json!({ "cluster": "mainnet-beta", "genesis": MAINNET_GENESIS });
        assert!(check_plan_cluster(&p, MAINNET_GENESIS).is_ok());
        assert!(check_plan_cluster(&p, DEVNET_GENESIS).is_err());
        assert!(check_plan_cluster(&json!({ "cluster": "localnet" }), MAINNET_GENESIS).is_err());
        assert!(check_plan_cluster(&json!({ "cluster": "localnet" }), "xyz").is_ok());
    }
}
