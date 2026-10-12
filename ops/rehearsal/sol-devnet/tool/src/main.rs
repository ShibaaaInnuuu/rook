//! rook-devnet: the operator's side of the Solana devnet rehearsal (docs/v9/rehearsal-sol-devnet.md). DEVNET OR A LOCAL VALIDATOR ONLY:
//! it refuses any RPC whose genesis hash is mainnet-beta's.
//!
//!   rook-devnet --rpc <url> --job <dir> --payer <keypair.json> <command> [args]
//!
//! Commands (each idempotent where it can be; state in <job>/state.json, the throwaway role keys in <job>/keys/, 0600, never printed):
//!   keys                                  the role keypairs (keeper, guardian, custodian, holder): created if absent, pubkeys printed
//!   fund [--sol-holder N] [--airdrop]     moves SOL from the payer to the role keys (or airdrops them: local validator / devnet faucet)
//!   plan                                  this job's three listing ids (the registry's next three: the program ids are fixed, so a
//!                                         second run on the same cluster lists after the first) and their PDAs, as JSON (for the
//!                                         canister's install argument, which names them before they exist)
//!   init --launch <file> [--canister <base58>] [--canister-taproot <tb1p…>] [--canister-evm <0x…>] [--frog-ids <file>]
//!                                         registry (owner = payer = the upgrade authority, the launch file's Bitcoin network), guardian,
//!                                         keeper, treasury, desk (launch values), three listings with wrapper + module (launch params):
//!                                           eth   EthNft (Milady on chain 1; custodian the canister, poster the keeper key: runbook §4)
//!                                           frog  Inscriptions "Frogs (canister)" (custodian the canister)
//!                                           drill Inscriptions "Frogs (drill)" (custodian the operator's custodian key: the program drill)
//!                                         with rook_backing's program-wide ["converter"] PDA allowed at init_registry (no 7-day wait;
//!                                         on a registry initialized before that change, it is proposed instead: accept-converter + dbc-back)
//!   post --listing eth|frog|drill|N --sol X [--conf C] [--depth D]   a floor post by the listing's poster (the keeper key)
//!   mint-units --listing N --sol X        the holder buys wX publicly (SOL in, units out): the module's net demand
//!   burn-units --listing N [--all]        the holder burns its wX back into SOL at the floor (recovers devnet SOL after a run)
//!   fund-reserve --listing N --sol X      the payer funds the module's surplus
//!   pay --to <base58> --sol X             the payer sends SOL (e.g. to the canister's Solana address: its fees and nonce rent)
//!   balance --of <base58>                 lamports at an address
//!   drill                                 listing 2, every custodian and holder step of the program on this cluster (see `drill`)
//!   refusals                              what must be refused, simulated (no fee): the network rules, roles, a second init
//!   proof --listing N --inscription <id>  the membership proof (hex) of an inscription of the listing's tree (for frog_offer / frog_proof)
//!   status                                every account, decoded
//!   accept-converter                      the owner accepts a pending converter (only for a registry initialized without it: 7 days)
//!   dbc-launch [--threshold-sol 2] [--no-wait]   Meteora DBC on this cluster: init_backing, register_partner (listing 0), a config with
//!                                         the claimer PDA as fee claimer, a coin launched, its vault opened, bought to the threshold,
//!                                         migrated to DAMM v2 (permissionless), then `back` (withdraw_migration_fee → rook_core convert,
//!                                         signed by the ["converter"] PDA) and redeem_backing, in the same run
//!   dbc-back                              `back` + redeem_backing again on the launched coin (fallback: a registry whose converter
//!                                         was proposed rather than set at init, after accept-converter)
//!   report                                the job's report (every step, signature, CU)
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread::sleep,
    time::{Duration, Instant},
};

use anchor_lang::{
    prelude::{AccountMeta, Pubkey},
    solana_program::{instruction::Instruction, system_instruction, system_program},
    AccountDeserialize, InstructionData, ToAccountMetas,
};
use anchor_spl::{
    associated_token::{get_associated_token_address, spl_associated_token_account::instruction as ata_ix},
    token::spl_token,
};
use base64::Engine;
use rook_core::{
    hashing::{keccak, leaf_of},
    instructions::{ListArgs, MintItemArgs},
    params::Params,
    seeds::*,
    state::*,
};
use serde_json::{json, Value};
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_keypair::Keypair;
use solana_message::{Message, VersionedMessage};
use solana_signer::Signer;
use solana_transaction::versioned::VersionedTransaction;

const SOL: u64 = 1_000_000_000;
const ONE: u64 = rook_core::units::ONE;
const MAINNET_GENESIS: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";
const DEVNET_GENESIS: &str = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";
/// Milady Maker on Ethereum (the EthNft listing's collection; nothing on Ethereum is touched by this tool).
const MILADY: &str = "0x5Af0D9827E0c53E4799BB226655A1de152A425a5";
/// A placeholder Ethereum custody when the canister's address is not given (program-only runs).
const EVM_PLACEHOLDER: &str = "0x00000000000000000000000000000000000000c5";
/// BIP341's NUMS output with the `tb` prefix (rook_core hashing tests): a valid signet/testnet4 taproot nobody spends.
const TB_NUMS: &str = "tb1p2zffkaxp5py4fdutfdsrt6t6tcrc5ks09rkfd428hlhf4n5q8tqqeygjde";
const BC_NUMS: &str = rook_core::hashing::BTC_BURN_ADDRESS;
/// A test destination on signet (a valid tb1p of a throwaway witness program; the drill never sends Bitcoin).
const TB_DEST: &str = "tb1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqp3mvzv";
/// Two synthetic inscription ids every tree carries (the drill mints these); real signet ids come from --frog-ids.
const SYNTH_FROGS: [&str; 3] = [
    "6fb976ab49dcec017f1e201e84395983204ae1a7c2abf7ced0a85d692e442799i0",
    "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90i1",
    "0f0e0d0c0b0a09080706050403020100f0e0d0c0b0a09080706050403020100fi2",
];

// ---- plumbing: args, state, keys ------------------------------------------------------------------------------------------------

struct Args {
    rpc: String,
    job: PathBuf,
    payer: PathBuf,
    cmd: String,
    rest: BTreeMap<String, String>,
}

fn args() -> Result<Args, String> {
    let mut v = std::env::args().skip(1).collect::<Vec<_>>();
    let mut m = BTreeMap::new();
    let mut cmd = None;
    let mut i = 0;
    while i < v.len() {
        let a = v[i].clone();
        if let Some(k) = a.strip_prefix("--") {
            let val = if i + 1 < v.len() && !v[i + 1].starts_with("--") {
                i += 1;
                v[i].clone()
            } else {
                "true".into()
            };
            m.insert(k.to_string(), val);
        } else if cmd.is_none() {
            cmd = Some(a);
        } else {
            return Err(format!("unexpected argument {a}"));
        }
        i += 1;
    }
    v.clear();
    let take = |m: &mut BTreeMap<String, String>, k: &str| m.remove(k).ok_or(format!("--{k} is required"));
    Ok(Args {
        rpc: take(&mut m, "rpc")?,
        job: PathBuf::from(take(&mut m, "job")?),
        payer: PathBuf::from(take(&mut m, "payer")?),
        cmd: cmd.ok_or("a command is required (see the header of src/main.rs)")?,
        rest: m,
    })
}

fn read_kp(p: &Path) -> Result<Keypair, String> {
    let s = fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let b: Vec<u8> = serde_json::from_str(&s).map_err(|e| format!("{}: not a keypair file: {e}", p.display()))?;
    Keypair::try_from(&b[..]).map_err(|e| format!("{}: {e}", p.display()))
}

/// A throwaway role key in the job directory: created 0600 if absent. The secret never leaves the file.
fn role_kp(job: &Path, name: &str) -> Result<Keypair, String> {
    let dir = job.join("keys");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let p = dir.join(format!("{name}.json"));
    if !p.exists() {
        let k = Keypair::new();
        let mut f = fs::OpenOptions::new().write(true).create_new(true).mode_0600().open(&p).map_err(|e| e.to_string())?;
        f.write_all(serde_json::to_string(&k.to_bytes().to_vec()).unwrap().as_bytes()).map_err(|e| e.to_string())?;
    }
    read_kp(&p)
}

trait Mode0600 {
    fn mode_0600(&mut self) -> &mut Self;
}
impl Mode0600 for fs::OpenOptions {
    fn mode_0600(&mut self) -> &mut Self {
        use std::os::unix::fs::OpenOptionsExt;
        self.mode(0o600)
    }
}

struct Job {
    dir: PathBuf,
    state: Value,
}

impl Job {
    fn open(dir: &Path) -> Result<Job, String> {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let p = dir.join("state.json");
        let state = if p.exists() { serde_json::from_str(&fs::read_to_string(&p).map_err(|e| e.to_string())?).map_err(|e| e.to_string())? } else { json!({ "steps": [] }) };
        Ok(Job { dir: dir.to_path_buf(), state })
    }
    fn save(&self) {
        let _ = fs::write(self.dir.join("state.json"), serde_json::to_string_pretty(&self.state).unwrap());
    }
    fn step(&mut self, kind: &str, name: &str, detail: &str, sig: Option<&str>, cu: Option<u64>) {
        println!("{kind} {name}: {detail}{}{}", cu.map(|c| format!(" [{c} CU]")).unwrap_or_default(), sig.map(|s| format!(" {s}")).unwrap_or_default());
        self.state["steps"].as_array_mut().unwrap().push(json!({ "kind": kind, "name": name, "detail": detail, "signature": sig, "cu": cu,
            "at": now_s() }));
        self.save();
    }
    fn get(&self, k: &str) -> Option<&Value> {
        self.state.get(k).filter(|v| !v.is_null())
    }
}

fn now_s() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

// ---- JSON-RPC (curl) --------------------------------------------------------------------------------------------------------------

struct Rpc {
    url: String,
}

impl Rpc {
    fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string();
        for attempt in 0..4 {
            let mut c = Command::new("curl")
                .args(["-sS", "-m", "60", "-H", "content-type: application/json", "--data-binary", "@-", &self.url])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| format!("curl: {e}"))?;
            c.stdin.take().unwrap().write_all(body.as_bytes()).map_err(|e| e.to_string())?;
            let out = c.wait_with_output().map_err(|e| e.to_string())?;
            let text = String::from_utf8_lossy(&out.stdout).to_string();
            match serde_json::from_str::<Value>(&text) {
                Ok(v) => {
                    if let Some(e) = v.get("error") {
                        // 429 / transient node errors: back off and retry
                        if e["code"] == json!(429) || text.contains("Too many requests") {
                            sleep(Duration::from_secs(2 + 3 * attempt));
                            continue;
                        }
                        return Err(format!("{method}: {e}"));
                    }
                    return Ok(v["result"].clone());
                }
                Err(_) => {
                    if attempt == 3 {
                        return Err(format!("{method}: not JSON: {} {}", text.chars().take(300).collect::<String>(), String::from_utf8_lossy(&out.stderr)));
                    }
                    sleep(Duration::from_secs(2 + 3 * attempt));
                }
            }
        }
        Err(format!("{method}: rate limited"))
    }

    fn account(&self, k: &Pubkey) -> Result<Option<(u64, Pubkey, Vec<u8>)>, String> {
        let v = self.call("getAccountInfo", json!([k.to_string(), { "encoding": "base64", "commitment": "confirmed" }]))?;
        let a = &v["value"];
        if a.is_null() {
            return Ok(None);
        }
        let data = base64::engine::general_purpose::STANDARD.decode(a["data"][0].as_str().unwrap_or("")).map_err(|e| e.to_string())?;
        let owner: Pubkey = a["owner"].as_str().unwrap_or("").parse().map_err(|_| "owner".to_string())?;
        Ok(Some((a["lamports"].as_u64().unwrap_or(0), owner, data)))
    }

    fn exists(&self, k: &Pubkey) -> bool {
        matches!(self.account(k), Ok(Some((l, _, _))) if l > 0)
    }

    fn get<T: AccountDeserialize>(&self, k: &Pubkey) -> Result<T, String> {
        let (_, _, d) = self.account(k)?.ok_or(format!("{k} does not exist"))?;
        T::try_deserialize(&mut d.as_slice()).map_err(|e| format!("{k}: {e}"))
    }

    fn balance(&self, k: &Pubkey) -> u64 {
        self.call("getBalance", json!([k.to_string(), { "commitment": "confirmed" }])).ok().and_then(|v| v["value"].as_u64()).unwrap_or(0)
    }

    /// The latest blockhash at `confirmed` and the last block height at which a transaction on it can land.
    fn blockhash(&self) -> Result<(solana_message::Hash, u64), String> {
        let v = self.call("getLatestBlockhash", json!([{ "commitment": "confirmed" }]))?;
        let h = v["value"]["blockhash"].as_str().unwrap_or("").parse().map_err(|_| "blockhash".to_string())?;
        Ok((h, v["value"]["lastValidBlockHeight"].as_u64().ok_or("lastValidBlockHeight")?))
    }

    fn block_height(&self) -> Result<u64, String> {
        self.call("getBlockHeight", json!([{ "commitment": "confirmed" }]))?.as_u64().ok_or_else(|| "getBlockHeight".into())
    }

    /// A signature's status, searched in the history too (a status-cache miss on one node of a load-balanced RPC is not "not landed").
    fn status(&self, sig: &str) -> Result<Value, String> {
        Ok(self.call("getSignatureStatuses", json!([[sig], { "searchTransactionHistory": true }]))?["value"][0].clone())
    }

    fn token_amount(&self, k: &Pubkey) -> u64 {
        match self.account(k) {
            Ok(Some((_, _, d))) if d.len() >= 72 => u64::from_le_bytes(d[64..72].try_into().unwrap()),
            _ => 0,
        }
    }
}

// ---- sending: simulate, sign, send, confirm; and refusals (simulated only) ---------------------------------------------------------

struct Sent {
    sig: String,
    cu: Option<u64>,
}

fn build(rpc: &Rpc, ixs: &[Instruction], signers: &[&Keypair]) -> Result<VersionedTransaction, String> {
    Ok(build_lv(rpc, ixs, signers)?.0)
}

/// The transaction and the last block height its blockhash is valid at.
fn build_lv(rpc: &Rpc, ixs: &[Instruction], signers: &[&Keypair]) -> Result<(VersionedTransaction, u64), String> {
    let mut all = vec![ComputeBudgetInstruction::set_compute_unit_limit(600_000)];
    all.extend_from_slice(ixs);
    let mut ks: Vec<&Keypair> = vec![];
    for s in signers {
        if !ks.iter().any(|k| k.pubkey() == s.pubkey()) {
            ks.push(s);
        }
    }
    let (hash, last_valid) = rpc.blockhash()?;
    let msg = Message::new_with_blockhash(&all, Some(&ks[0].pubkey()), &hash);
    Ok((VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &ks).map_err(|e| format!("sign: {e}"))?, last_valid))
}

fn b64(tx: &VersionedTransaction) -> String {
    base64::engine::general_purpose::STANDARD.encode(bincode_tx(tx))
}

fn bincode_tx(tx: &VersionedTransaction) -> Vec<u8> {
    // wire format: shortvec signatures, then the serialized message
    let mut out = shortvec(tx.signatures.len());
    for s in &tx.signatures {
        out.extend_from_slice(s.as_ref());
    }
    out.extend(tx.message.serialize());
    out
}

fn shortvec(n: usize) -> Vec<u8> {
    let mut v = vec![];
    let mut x = n;
    loop {
        let mut b = (x & 0x7f) as u8;
        x >>= 7;
        if x != 0 {
            b |= 0x80;
        }
        v.push(b);
        if x == 0 {
            return v;
        }
    }
}

/// Simulated: Ok(logs, CU) or Err(the error and the logs).
fn simulate(rpc: &Rpc, tx: &VersionedTransaction) -> Result<(Vec<String>, Option<u64>), String> {
    let v = rpc.call("simulateTransaction", json!([b64(tx), { "encoding": "base64", "commitment": "confirmed", "sigVerify": false }]))?;
    let logs: Vec<String> = v["value"]["logs"].as_array().map(|a| a.iter().filter_map(|l| l.as_str().map(String::from)).collect()).unwrap_or_default();
    if !v["value"]["err"].is_null() {
        let tail: Vec<&String> = logs.iter().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect();
        return Err(format!("{} | {}", v["value"]["err"], tail.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" | ")));
    }
    Ok((logs, v["value"]["unitsConsumed"].as_u64()))
}

/// Simulate, sign, send, then wait for `confirmed` until the blockhash has really expired (the cluster's block height past the
/// transaction's `lastValidBlockHeight`), not a fixed time: devnet's RPC is load-balanced, a node's status cache can miss a signature
/// that landed (run 3, 2026-10-09: `back` landed 9 s after it was sent and the tool reported it unconfirmed after 90 s). Statuses are
/// read with the history searched, and once more after the expiry before anything is called "not landed".
fn send(rpc: &Rpc, ixs: &[Instruction], signers: &[&Keypair]) -> Result<Sent, String> {
    let (tx, last_valid) = build_lv(rpc, ixs, signers)?;
    simulate(rpc, &tx)?;
    let sig = tx.signatures[0].to_string();
    let raw = b64(&tx);
    rpc.call("sendTransaction", json!([raw, { "encoding": "base64", "skipPreflight": true, "maxRetries": 5 }]))?;
    let t0 = Instant::now();
    let mut resent = Instant::now();
    loop {
        sleep(Duration::from_millis(1500));
        // a read that fails (rate limit, a node's hiccup) is not a verdict: the next poll asks again
        if let Ok(s) = rpc.status(&sig) {
            if let Some(true) = landed(&s, &sig)? {
                break;
            }
        }
        let expired = rpc.block_height().is_ok_and(|h| h > last_valid);
        if expired || t0.elapsed() > Duration::from_secs(300) {
            // the blockhash is past: the transaction can no longer land; one last look, history included
            sleep(Duration::from_secs(3));
            if let Some(true) = landed(&rpc.status(&sig)?, &sig)? {
                break;
            }
            return Err(format!("not landed: its blockhash expired (block height past {last_valid}) after {} s ({sig}); nothing \
                moved: send again", t0.elapsed().as_secs()));
        }
        if resent.elapsed() > Duration::from_secs(10) {
            // the same bytes again (the same signature: it lands at most once)
            let _ = rpc.call("sendTransaction", json!([raw, { "encoding": "base64", "skipPreflight": true, "maxRetries": 0 }]));
            resent = Instant::now();
        }
    }
    let cu = rpc
        .call("getTransaction", json!([sig, { "encoding": "json", "commitment": "confirmed", "maxSupportedTransactionVersion": 0 }]))
        .ok()
        .and_then(|v| v["meta"]["computeUnitsConsumed"].as_u64());
    Ok(Sent { sig, cu })
}

/// A status: `Some(true)` landed at `confirmed` or deeper, `Some(false)` seen but not yet confirmed, `None` not seen; an error if it
/// landed with an error.
fn landed(s: &Value, sig: &str) -> Result<Option<bool>, String> {
    if s.is_null() {
        return Ok(None);
    }
    if !s["err"].is_null() {
        return Err(format!("landed with an error {} ({sig})", s["err"]));
    }
    let c = s["confirmationStatus"].as_str().unwrap_or("");
    Ok(Some(c == "confirmed" || c == "finalized"))
}

/// Simulated only; Ok(the refusal) when it is refused with `want` in its error (a custom code, e.g. `"Custom":6012`), Err otherwise.
fn refused(rpc: &Rpc, ixs: &[Instruction], signers: &[&Keypair], want: &str) -> Result<String, String> {
    let tx = build(rpc, ixs, signers)?;
    match simulate(rpc, &tx) {
        Ok(_) => Err("ACCEPTED in simulation (expected a refusal)".into()),
        Err(e) if e.contains(want) => Ok(e.split(" | ").next().unwrap_or("").to_string()),
        Err(e) => Err(format!("refused, but not with {want}: {e}")),
    }
}

/// A rook_core error as `simulateTransaction` reports it (`{"InstructionError":[i,{"Custom":6001}]}`).
fn code(e: rook_core::error::RookError) -> String {
    format!("\"Custom\":{}", e as u32 + 6000)
}

// ---- rook_core accounts ---------------------------------------------------------------------------------------------------------

fn pda(seeds: &[&[u8]]) -> Pubkey {
    Pubkey::find_program_address(seeds, &rook_core::ID).0
}

#[derive(Clone, Copy)]
struct Col {
    id: u32,
    listing: Pubkey,
    wrapper: Pubkey,
    wx_mint: Pubkey,
    item_set: Pubkey,
    module: Pubkey,
    vault: Pubkey,
    module_wx: Pubkey,
}

fn col(id: u32) -> Col {
    let listing = pda(&[LISTING, &id.to_le_bytes()]);
    let wrapper = pda(&[WRAPPER, listing.as_ref()]);
    let module = pda(&[MODULE, listing.as_ref()]);
    Col {
        id,
        listing,
        wrapper,
        wx_mint: pda(&[WX_MINT, wrapper.as_ref()]),
        item_set: pda(&[ITEMS, wrapper.as_ref()]),
        module,
        vault: pda(&[MODULE_VAULT, module.as_ref()]),
        module_wx: pda(&[MODULE_WX, module.as_ref()]),
    }
}

fn ix<D: InstructionData, A: ToAccountMetas>(data: D, accounts: A) -> Instruction {
    Instruction::new_with_bytes(rook_core::ID, &data.data(), accounts.to_account_metas(None))
}

fn registry() -> Pubkey {
    pda(&[REGISTRY])
}

fn program_data(program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[program.as_ref()], &anchor_lang::solana_program::bpf_loader_upgradeable::ID).0
}

// ---- the launch file ------------------------------------------------------------------------------------------------------------

fn u64s(v: &Value) -> Result<u64, String> {
    match v {
        Value::String(s) => s.parse().map_err(|_| format!("not a u64: {s}")),
        Value::Number(n) => n.as_u64().ok_or(format!("not a u64: {n}")),
        _ => Err(format!("not a u64: {v}")),
    }
}

struct Launch {
    network: BitcoinNetwork,
    params: Params,
    desk: DeskParams,
    rate: String,
    path: String,
}

fn launch(path: &str) -> Result<Launch, String> {
    let f: Value = serde_json::from_str(&fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?).map_err(|e| format!("{path}: {e}"))?;
    let network = match f["init"]["init_registry"]["bitcoin_network"].as_str() {
        Some("Mainnet") => BitcoinNetwork::Mainnet,
        Some("Signet") => BitcoinNetwork::Signet,
        Some("Testnet4") => BitcoinNetwork::Testnet4,
        other => return Err(format!("{path}: init.init_registry.bitcoin_network {other:?}")),
    };
    let p = &f["init"]["init_module"]["params"];
    let g = |k: &str| -> Result<u64, String> { u64s(&p[k]).map_err(|e| format!("params.{k}: {e}")) };
    let params = Params {
        base_spread_bps: g("base_spread_bps")? as u16,
        vol_weight_bps: g("vol_weight_bps")? as u16,
        stale_spread_bps: g("stale_spread_bps")? as u16,
        stress_spread_bps: g("stress_spread_bps")? as u16,
        max_spread_bps: g("max_spread_bps")? as u16,
        max_move_bps: g("max_move_bps")? as u16,
        conv_bps: g("conv_bps")? as u16,
        buffer_bps: g("buffer_bps")? as u16,
        low_confidence_spread_bps: g("low_confidence_spread_bps")? as u16,
        full_confidence_bps: g("full_confidence_bps")? as u16,
        min_confidence_bps: g("min_confidence_bps")? as u16,
        max_items_per_day: g("max_items_per_day")? as u16,
        reserve_bps: g("reserve_bps")? as u16,
        fill_band_bps: g("fill_band_bps")? as u16,
        mint_per_hour: g("mint_per_hour")?,
        burn_per_hour: g("burn_per_hour")?,
        max_public_eth_backed: g("max_public_eth_backed")?,
        max_eth_backed: g("max_eth_backed")?,
        revenue_target: g("revenue_target")?,
    };
    let d = &f["init"]["init_desk"];
    let desk = DeskParams { bond: u64s(&d["bond"])?, min_bond: u64s(&d["min_bond"])?, max_bond: u64s(&d["max_bond"])?, window: u64s(&d["window"])? as i64 };
    Ok(Launch { network, params, desk, rate: f["rate"]["sol_per_eth"].as_str().unwrap_or("?").to_string(), path: path.into() })
}

// ---- the inscription tree (sorted-pair keccak, as rook_core's verify_proof) ---------------------------------------------------------

fn pair(a: [u8; 32], b: [u8; 32]) -> [u8; 32] {
    if a <= b { keccak(&[&a, &b]) } else { keccak(&[&b, &a]) }
}

/// The root and every leaf's proof: levels pairwise, an odd node carried up unchanged.
fn tree(ids: &[String]) -> ([u8; 32], Vec<Vec<[u8; 32]>>) {
    let mut level: Vec<[u8; 32]> = ids.iter().map(|i| leaf_of(i)).collect();
    let mut pos: Vec<usize> = (0..ids.len()).collect();
    let mut proofs: Vec<Vec<[u8; 32]>> = vec![vec![]; ids.len()];
    while level.len() > 1 {
        for (leaf, p) in pos.iter_mut().enumerate() {
            let sib = *p ^ 1;
            if sib < level.len() {
                proofs[leaf].push(level[sib]);
            }
            *p /= 2;
        }
        level = level.chunks(2).map(|c| if c.len() == 2 { pair(c[0], c[1]) } else { c[0] }).collect();
    }
    (level[0], proofs)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn frog_ids(job: &Job) -> Vec<String> {
    job.get("frog_ids").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

// ---- the commands ---------------------------------------------------------------------------------------------------------------

struct Ctx {
    rpc: Rpc,
    job: Job,
    payer: Keypair,
    a: BTreeMap<String, String>,
}

impl Ctx {
    fn role(&self, name: &str) -> Result<Keypair, String> {
        role_kp(&self.job.dir, name)
    }
    fn opt(&self, k: &str) -> Option<String> {
        self.a.get(k).cloned()
    }
    fn num(&self, k: &str, d: f64) -> Result<f64, String> {
        self.a.get(k).map(|s| s.parse::<f64>().map_err(|_| format!("--{k} {s}"))).unwrap_or(Ok(d))
    }
    /// This job's listing ids (`plan`): eth, frog, drill.
    fn ids(&self) -> Result<(u32, u32, u32), String> {
        let l = self.job.get("listings").ok_or("run `plan` first: this job has no listing ids")?;
        let g = |k: &str| l[k].as_u64().map(|x| x as u32).ok_or(format!("listings.{k}"));
        Ok((g("eth")?, g("frog")?, g("drill")?))
    }
    fn listing(&self) -> Result<Col, String> {
        let (e, f, d) = self.ids()?;
        let v = self.a.get("listing").ok_or("--listing eth|frog|drill|N")?;
        Ok(col(match v.as_str() {
            "eth" => e,
            "frog" => f,
            "drill" => d,
            n => n.parse().map_err(|_| format!("--listing {n}"))?,
        }))
    }
    /// Sends and records; a failure is recorded and returned.
    fn go(&mut self, name: &str, ixs: &[Instruction], signers: &[&Keypair]) -> Result<Sent, String> {
        match send(&self.rpc, ixs, signers) {
            Ok(s) => {
                self.job.step("PASS", name, "landed", Some(&s.sig), s.cu);
                Ok(s)
            }
            Err(e) => {
                self.job.step("FAIL", name, &e, None, None);
                Err(format!("{name}: {e}"))
            }
        }
    }
    /// Runs `f` only when `done` is false (idempotent init); records SKIP otherwise.
    fn once(&mut self, name: &str, done: bool, ixs: &[Instruction], signers: &[&Keypair]) -> Result<(), String> {
        if done {
            self.job.step("SKIP", name, "already done on this cluster", None, None);
            return Ok(());
        }
        self.go(name, ixs, signers).map(|_| ())
    }
    fn expect_refused(&mut self, name: &str, ixs: &[Instruction], signers: &[&Keypair], want: &str) {
        match refused(&self.rpc, ixs, signers, want) {
            Ok(e) => self.job.step("PASS", name, &format!("refused as expected: {e}"), None, None),
            Err(e) => self.job.step("FAIL", name, &e, None, None),
        }
    }
}

fn check_cluster(rpc: &Rpc) -> Result<String, String> {
    let g = rpc.call("getGenesisHash", json!([]))?;
    let g = g.as_str().unwrap_or("").to_string();
    if g == MAINNET_GENESIS {
        return Err("refused: this RPC serves mainnet-beta; the rehearsal runs on devnet or a local validator only".into());
    }
    Ok(if g == DEVNET_GENESIS { "devnet".into() } else { format!("local or other ({g})") })
}

fn cmd_keys(c: &mut Ctx) -> Result<(), String> {
    for r in ["keeper", "guardian", "custodian", "holder"] {
        let k = c.role(r)?;
        println!("{r}: {} ({} lamports)", k.pubkey(), c.rpc.balance(&k.pubkey()));
        c.job.state["roles"][r] = json!(k.pubkey().to_string());
    }
    println!("payer (owner, upgrade authority): {} ({} lamports)", c.payer.pubkey(), c.rpc.balance(&c.payer.pubkey()));
    c.job.state["roles"]["payer"] = json!(c.payer.pubkey().to_string());
    c.job.save();
    Ok(())
}

fn cmd_fund(c: &mut Ctx) -> Result<(), String> {
    let holder = (c.num("sol-holder", 3.0)? * SOL as f64) as u64;
    let plan = [("keeper", SOL / 5), ("guardian", SOL / 20), ("custodian", SOL / 2), ("holder", holder)];
    for (r, want) in plan {
        let k = c.role(r)?;
        let have = c.rpc.balance(&k.pubkey());
        if have >= want {
            c.job.step("SKIP", &format!("fund {r}"), &format!("holds {have} lamports"), None, None);
            continue;
        }
        if c.a.contains_key("airdrop") {
            let sig = c.rpc.call("requestAirdrop", json!([k.pubkey().to_string(), want - have]))?;
            sleep(Duration::from_secs(3));
            c.job.step("PASS", &format!("airdrop {r}"), &format!("{} lamports", want - have), sig.as_str(), None);
        } else {
            let p = c.payer.insecure_clone();
            c.go(&format!("fund {r}"), &[system_instruction::transfer(&p.pubkey(), &k.pubkey(), want - have)], &[&p])?;
        }
    }
    Ok(())
}

fn cmd_plan(c: &mut Ctx) -> Result<(), String> {
    if c.job.get("listings").is_none() {
        let base = if c.rpc.exists(&registry()) { c.rpc.get::<Registry>(&registry())?.listing_count } else { 0 };
        c.job.state["listings"] = json!({ "eth": base, "frog": base + 1, "drill": base + 2 });
        c.job.save();
    }
    let (e, f, d) = c.ids()?;
    let one = |id: u32| { let k = col(id); json!({ "id": id, "listing": k.listing.to_string(), "wrapper": k.wrapper.to_string(), "module": k.module.to_string(),
        "wx_mint": k.wx_mint.to_string() }) };
    let out = json!({ "rook_core": rook_core::ID.to_string(), "rook_backing": rook_backing::ID.to_string(), "registry": registry().to_string(),
        "eth": one(e), "frog": one(f), "drill": one(d) });
    fs::write(c.job.dir.join("plan.json"), serde_json::to_string_pretty(&out).unwrap()).map_err(|e| e.to_string())?;
    println!("{out}");
    Ok(())
}

fn init_module_ix(owner: &Pubkey, k: &Col, p: Params) -> Instruction {
    ix(
        rook_core::instruction::InitModule { params: p },
        rook_core::accounts::InitModule {
            owner: *owner,
            payer: *owner,
            registry: registry(),
            listing: k.listing,
            wrapper: k.wrapper,
            wx_mint: k.wx_mint,
            module: k.module,
            module_vault: k.vault,
            module_wx: k.module_wx,
            token_program: spl_token::ID,
            system_program: system_program::ID,
        },
    )
}

fn cmd_init(c: &mut Ctx) -> Result<(), String> {
    let l = launch(&c.opt("launch").ok_or("--launch <launch file> (ops/solana/launch-params.mjs)")?)?;
    let o = c.payer.insecure_clone();
    let (keeper, guardian, custodian) = (c.role("keeper")?, c.role("guardian")?, c.role("custodian")?);
    let canister: Option<Pubkey> = c.opt("canister").map(|s| s.parse().map_err(|_| format!("--canister {s}: not base58"))).transpose()?;
    let canister_taproot = c.opt("canister-taproot");
    let canister_evm = c.opt("canister-evm").unwrap_or_else(|| EVM_PLACEHOLDER.into());
    let mut ids: Vec<String> = SYNTH_FROGS.iter().map(|s| s.to_string()).collect();
    if let Some(f) = c.opt("frog-ids") {
        for line in fs::read_to_string(&f).map_err(|e| format!("{f}: {e}"))?.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            ids.push(line.to_string());
        }
    }
    let (ie, ifr, idr) = c.ids()?;
    if let Some(prev) = c.job.get("frog_ids") {
        if prev != &json!(ids) && c.rpc.exists(&col(ifr).listing) {
            return Err("refused: the listings exist with another inscription set (their roots are fixed); use the same --frog-ids".into());
        }
    }
    c.job.state["frog_ids"] = json!(ids);
    c.job.state["launch"] = json!({ "path": l.path, "rate_sol_per_eth": l.rate, "bitcoin_network": format!("{:?}", l.network) });
    let (root, _) = tree(&ids);
    c.job.state["frog_root"] = json!(hex(&root));
    c.job.save();
    let reg = registry();

    // the registry, by the upgrade authority (= the payer here: the deploy keypair)
    let done = c.rpc.exists(&reg);
    // rook_backing's ONE converter, the program-wide ["converter"] PDA, allowed at init (no CHANGE_DELAY: only the upgrade authority initializes)
    let converter = dbcx::converter();
    let i = ix(
        rook_core::instruction::InitRegistry { owner: o.pubkey(), bitcoin_network: l.network, initial_converters: vec![converter] },
        rook_core::accounts::InitRegistry { authority: o.pubkey(), payer: o.pubkey(), registry: reg, program_data: program_data(&rook_core::ID),
            system_program: system_program::ID },
    );
    c.once(&format!("init_registry (Bitcoin {:?}, initial converter rook_backing {converter})", l.network), done, &[i], &[&o])?;
    let r: Registry = c.rpc.get(&reg)?;
    if r.bitcoin_network != l.network {
        return Err(format!("the registry on this cluster serves Bitcoin {:?}, the launch file names {:?}", r.bitcoin_network, l.network));
    }
    let admin = |d: &dyn Fn() -> Vec<u8>| Instruction::new_with_bytes(rook_core::ID, &d(), rook_core::accounts::RegistryAdmin { signer: o.pubkey(), registry: reg }.to_account_metas(None));
    let g = guardian.pubkey();
    c.once("set_guardian", r.guardian == g, &[admin(&|| rook_core::instruction::SetGuardian { guardian: g }.data())], &[&o])?;
    let k = keeper.pubkey();
    c.once("propose_roles (keeper: the keeper service's key, played by an operator keypair)", r.keeper == k,
        &[admin(&|| rook_core::instruction::ProposeRoles { keeper: k }.data())], &[&o])?;
    let treasury = pda(&[TREASURY]);
    c.once("init_treasury", c.rpc.exists(&treasury), &[ix(rook_core::instruction::InitTreasury {},
        rook_core::accounts::InitTreasury { owner: o.pubkey(), payer: o.pubkey(), registry: reg, treasury, system_program: system_program::ID })], &[&o])?;
    let desk = pda(&[DESK]);
    c.once(&format!("init_desk (bond {} lamports in [{}, {}], window {} s)", l.desk.bond, l.desk.min_bond, l.desk.max_bond, l.desk.window),
        c.rpc.exists(&desk), &[ix(rook_core::instruction::InitDesk { params: l.desk },
            rook_core::accounts::InitDesk { owner: o.pubkey(), payer: o.pubkey(), registry: reg, desk, system_program: system_program::ID })], &[&o])?;

    // the three listings
    let tb_custody = |net: BitcoinNetwork| if net == BitcoinNetwork::Mainnet { BC_NUMS } else { TB_NUMS };
    let can = canister.unwrap_or(custodian.pubkey());
    let plan: [(ListingKind, String, String, Pubkey, Pubkey); 3] = [
        (ListingKind::EthNft { chain_id: 1, collection: rook_core::hashing::parse_evm_address(MILADY).unwrap() }, "Milady (rehearsal)".into(),
            canister_evm.clone(), can, keeper.pubkey()),
        (ListingKind::Inscriptions { membership_root: root }, "Frogs (canister)".into(),
            canister_taproot.clone().unwrap_or_else(|| tb_custody(l.network).into()), can, Pubkey::default()),
        (ListingKind::Inscriptions { membership_root: root }, "Frogs (drill)".into(), tb_custody(l.network).into(), custodian.pubkey(), Pubkey::default()),
    ];
    for (((kind, name, custody, cust, poster), id), symbol) in plan.into_iter().zip([ie, ifr, idr]).zip(["wMILADY", "wBFROG", "wDRILL"]) {
        let k = col(id);
        let r: Registry = c.rpc.get(&reg)?;
        if !c.rpc.exists(&k.listing) {
            if r.listing_count != id {
                return Err(format!("the registry has {} listings; this job's listing {id} (plan) would not be at its PDA: someone listed in between, \
                    start a new job (a new --job dir) and install the canister with its new plan", r.listing_count));
            }
            c.go(&format!("list_collection {id} {name} (custodian {cust}, custody {custody})"), &[ix(
                rook_core::instruction::ListCollection { args: ListArgs { kind, name: name.clone(), custody: custody.clone(), custodian: cust, poster } },
                rook_core::accounts::ListCollection { owner: o.pubkey(), payer: o.pubkey(), registry: reg, listing: k.listing, system_program: system_program::ID },
            )], &[&o])?;
        } else {
            let have: Listing = c.rpc.get(&k.listing)?;
            c.job.step("SKIP", &format!("list_collection {id}"), &format!("listed: {} (custodian {})", have.name, have.custodian), None, None);
        }
        // the wX mint's Metaplex metadata (programs commit 6588b81): name / symbol; uri empty (settable later by set_wx_metadata)
        let meta = rook_core::WxMetadata { name: name.clone(), symbol: symbol.into(), uri: String::new() };
        let mpl = rook_core::metadata::MPL_TOKEN_METADATA_ID;
        let metadata = Pubkey::find_program_address(&[rook_core::metadata::METADATA_SEED, mpl.as_ref(), k.wx_mint.as_ref()], &mpl).0;
        c.once(&format!("init_wrapper {id} ({symbol})"), c.rpc.exists(&k.wrapper), &[ix(rook_core::instruction::InitWrapper { meta }, rook_core::accounts::InitWrapper {
            owner: o.pubkey(), payer: o.pubkey(), registry: reg, listing: k.listing, wrapper: k.wrapper, wx_mint: k.wx_mint, item_set: k.item_set,
            metadata, token_metadata_program: mpl, token_program: spl_token::ID, system_program: system_program::ID })], &[&o])?;
        c.once(&format!("init_module {id} (revenue_target {} lamports from {})", l.params.revenue_target, l.path), c.rpc.exists(&k.module),
            &[init_module_ix(&o.pubkey(), &k, l.params)], &[&o])?;
        let t: Treasury = c.rpc.get(&treasury)?;
        c.once(&format!("add_collection {id} (treasury)"), t.collections.contains(&k.module), &[ix(rook_core::instruction::AddCollection {},
            rook_core::accounts::AddCollection { owner: o.pubkey(), registry: reg, treasury, module: k.module })], &[&o])?;
    }
    // the converter is allowed from init_registry; a registry initialized before that change (an earlier devnet run) gets it proposed
    // instead, accepted after 7 days (`accept-converter`, then `dbc-back`)
    let r: Registry = c.rpc.get(&reg)?;
    if r.is_converter(&converter) {
        c.job.step("PASS", "rook_backing's [\"converter\"] PDA is an allowed converter from init_registry", &converter.to_string(), None, None);
    } else {
        c.once(&format!("propose_converter (rook_backing converter {converter}: this registry predates initial converters; accept on or after +7 d)"),
            r.pending_converter == converter, &[admin(&|| rook_core::instruction::ProposeConverter { converter }.data())], &[&o])?;
    }
    c.job.state["converter"] = json!(converter.to_string());
    c.job.state["claimer"] = json!(dbcx::claimer(&col(ie).listing).to_string());
    c.job.state["canister"] = json!(canister.map(|k| k.to_string()));
    c.job.save();
    println!("frog root {} over {} inscription ids; listings: eth {ie} {} / frog {ifr} {} / drill {idr} {}", hex(&root), ids.len(), col(ie).listing,
        col(ifr).listing, col(idr).listing);
    Ok(())
}

fn post_ix(poster: &Pubkey, k: &Col, price: u64, conf: u16, depth: u32) -> Instruction {
    ix(rook_core::instruction::PostFloor { price, confidence_bps: conf, depth_items: depth },
        rook_core::accounts::PostFloor { poster: *poster, registry: registry(), listing: k.listing, module: k.module })
}

fn cmd_post(c: &mut Ctx) -> Result<(), String> {
    let k = c.listing()?;
    let price = (c.num("sol", 1.5)? * SOL as f64) as u64;
    let keeper = c.role("keeper")?;
    let (conf, depth) = (c.num("conf", 9000.0)? as u16, c.num("depth", 10.0)? as u32);
    c.go(&format!("post_floor listing {} ({price} lamports per unit, signed by the keeper key)", k.id), &[post_ix(&keeper.pubkey(), &k, price, conf, depth)], &[&keeper])?;
    let m: Module = c.rpc.get(&k.module)?;
    println!("module {}: accepted {} target {} posted_at {}", k.module, m.accepted, m.target, m.posted_at);
    Ok(())
}

fn ata_create(payer: &Pubkey, owner: &Pubkey, mint: &Pubkey) -> Instruction {
    ata_ix::create_associated_token_account_idempotent(payer, owner, mint, &spl_token::ID)
}

fn mint_ix(buyer: &Pubkey, k: &Col, dest: &Pubkey, lamports: u64) -> Instruction {
    ix(rook_core::instruction::Mint { lamports, min_units: 0 }, rook_core::accounts::MintUnits {
        buyer: *buyer, registry: registry(), listing: k.listing, module: k.module, module_vault: k.vault, wrapper: k.wrapper, wx_mint: k.wx_mint,
        dest: *dest, token_program: spl_token::ID, system_program: system_program::ID })
}

fn cmd_mint_units(c: &mut Ctx) -> Result<(), String> {
    let k = c.listing()?;
    let h = c.role("holder")?;
    let lamports = (c.num("sol", 1.6)? * SOL as f64) as u64;
    let acc = get_associated_token_address(&h.pubkey(), &k.wx_mint);
    c.go(&format!("mint listing {} ({lamports} lamports in, the holder's public buy)", k.id),
        &[ata_create(&h.pubkey(), &h.pubkey(), &k.wx_mint), mint_ix(&h.pubkey(), &k, &acc, lamports)], &[&h])?;
    let m: Module = c.rpc.get(&k.module)?;
    println!("holder wX {} base units; module backing {} reserve {} eth_backed? see status", c.rpc.token_amount(&acc), m.backing, m.reserve);
    Ok(())
}

fn cmd_burn_units(c: &mut Ctx) -> Result<(), String> {
    let k = c.listing()?;
    let h = c.role("holder")?;
    let acc = get_associated_token_address(&h.pubkey(), &k.wx_mint);
    let have = c.rpc.token_amount(&acc);
    let want = if c.a.contains_key("all") { have } else { have.min((c.num("units", 0.1)? * ONE as f64) as u64) };
    if want == 0 {
        c.job.step("SKIP", &format!("burn listing {}", k.id), "the holder holds no wX", None, None);
        return Ok(());
    }
    // rook_core burns only LIVE units: ETH-backed less the one in transit (`Module::live_units`; `burn` refuses NotEnoughBacked
    // above it). An advance out (the canister's, until return_advance or settle_advance) holds one whole unit back: run 3, 2026-10-09,
    // `--all` asked for 1,797,304,043 with 1,000,000,000 in transit and was refused.
    let m: Module = c.rpc.get(&k.module)?;
    let w: Wrapper = c.rpc.get(&k.wrapper)?;
    let live = m.live_units(w.eth_backed);
    let units = want.min(live);
    if units < want {
        c.job.step("INFO", &format!("burn listing {}: {} of {want} base units are burnable", k.id, units),
            &format!("eth_backed {} − transit {} (an advance out: back with return_advance or settle_advance); the rest stays wX",
                w.eth_backed, m.transit_units), None, None);
    }
    if units == 0 {
        c.job.step("SKIP", &format!("burn listing {}", k.id), "no live units (all in transit)", None, None);
        return Ok(());
    }
    c.go(&format!("burn {units} base units of listing {} (holder)", k.id), &[ix(rook_core::instruction::Burn { units, min_lamports: 0 }, rook_core::accounts::BurnUnits {
        owner: h.pubkey(), registry: registry(), listing: k.listing, module: k.module, module_vault: k.vault, wrapper: k.wrapper, wx_mint: k.wx_mint,
        owner_wx: acc, to: h.pubkey(), token_program: spl_token::ID })], &[&h])?;
    Ok(())
}

fn cmd_pay(c: &mut Ctx) -> Result<(), String> {
    let to: Pubkey = c.opt("to").ok_or("--to <base58>")?.parse().map_err(|_| "--to: not base58")?;
    let lamports = (c.num("sol", 0.1)? * SOL as f64) as u64;
    let p = c.payer.insecure_clone();
    c.go(&format!("pay {lamports} lamports to {to}"), &[system_instruction::transfer(&p.pubkey(), &to, lamports)], &[&p])?;
    Ok(())
}

fn cmd_balance(c: &mut Ctx) -> Result<(), String> {
    let of: Pubkey = c.opt("of").ok_or("--of <base58>")?.parse().map_err(|_| "--of: not base58")?;
    println!("{}", c.rpc.balance(&of));
    Ok(())
}

fn cmd_fund_reserve(c: &mut Ctx) -> Result<(), String> {
    let k = c.listing()?;
    let lamports = (c.num("sol", 0.5)? * SOL as f64) as u64;
    let p = c.payer.insecure_clone();
    c.go(&format!("fund_reserve listing {} ({lamports} lamports)", k.id), &[ix(rook_core::instruction::FundReserve { lamports },
        rook_core::accounts::FundReserve { payer: p.pubkey(), module: k.module, module_vault: k.vault, system_program: system_program::ID })], &[&p])?;
    Ok(())
}

fn mint_item_ix(custodian: &Pubkey, k: &Col, item_count: u32, args: MintItemArgs, dest: &Pubkey) -> Instruction {
    let item = pda(&[ITEM, k.wrapper.as_ref(), &item_count.to_le_bytes()]);
    let held = pda(&[HELD, k.wrapper.as_ref(), &args.key]);
    let source = pda(&[SOURCE, k.wrapper.as_ref(), &args.source]);
    ix(rook_core::instruction::MintItem { args }, rook_core::accounts::MintItem {
        custodian: *custodian, listing: k.listing, wrapper: k.wrapper, wx_mint: k.wx_mint, item_set: k.item_set, item, held, source, dest: *dest,
        token_program: spl_token::ID, system_program: system_program::ID })
}

fn frog_args(id: &str, proof: Vec<[u8; 32]>, tx: [u8; 32], vout: u32) -> MintItemArgs {
    MintItemArgs { key: leaf_of(id), source: keccak(&[&tx, &vout.to_be_bytes()]), tx, vout, deposit: false, inscription_id: id.into(), proof }
}

fn ticket_pda(k: &Col, id: u64) -> Pubkey {
    pda(&[TICKET, k.wrapper.as_ref(), &id.to_le_bytes()])
}

fn redeem_ix(owner: &Pubkey, k: &Col, owner_wx: &Pubkey, ticket: u64, d: Destination) -> Instruction {
    ix(rook_core::instruction::Redeem { destination: d }, rook_core::accounts::Redeem {
        owner: *owner, listing: k.listing, wrapper: k.wrapper, wx_mint: k.wx_mint, owner_wx: *owner_wx, ticket: ticket_pda(k, ticket),
        token_program: spl_token::ID, system_program: system_program::ID })
}

fn prove_ix(keeper: &Pubkey, round: u64, randomness: [u8; 32]) -> Instruction {
    ix(rook_core::instruction::ProveRound { round, randomness }, rook_core::accounts::ProveRound {
        keeper: *keeper, payer: *keeper, registry: registry(), drand_round: pda(&[DRAND, &round.to_le_bytes()]), system_program: system_program::ID })
}

fn resolve_ix(k: &Col, ticket: u64, round: u64) -> Instruction {
    ix(rook_core::instruction::Resolve {}, rook_core::accounts::Resolve {
        wrapper: k.wrapper, item_set: k.item_set, ticket: ticket_pda(k, ticket), drand_round: Some(pda(&[DRAND, &round.to_le_bytes()])) })
}

fn custodian_ticket_ix<D: InstructionData>(d: D, custodian: &Pubkey, k: &Col, ticket: u64) -> Instruction {
    ix(d, rook_core::accounts::CustodianTicket { custodian: *custodian, listing: k.listing, wrapper: k.wrapper, ticket: ticket_pda(k, ticket) })
}

/// Listing 2, every step of the program a custodian (here an operator key: the canister's role) and a holder take, on THIS cluster's
/// binary: floor post (keeper), public mint, advance → return_advance, advance → settle_advance, mint_item with a Merkle proof (×2),
/// redeem to a signet taproot address (a mainnet one refused), keeper-proven drand round, resolve, note_sent, record_delivery, a second
/// ticket marked undeliverable, a burn of units, then the books and the wX ledger checked.
fn cmd_drill(c: &mut Ctx) -> Result<(), String> {
    let k = col(c.ids()?.2);
    let (keeper, cust, h) = (c.role("keeper")?, c.role("custodian")?, c.role("holder")?);
    let l: Listing = c.rpc.get(&k.listing)?;
    if l.custodian != cust.pubkey() {
        return Err("the drill listing's custodian is not this job's custodian key".into());
    }
    let ids = frog_ids(&c.job);
    let (root, proofs) = tree(&ids);
    if l.kind != (ListingKind::Inscriptions { membership_root: root }) {
        return Err("the drill listing's root is not this job's tree".into());
    }
    let price = (c.num("floor-sol", 1.2)? * SOL as f64) as u64;
    // 1. the keeper's post (the keeper service's role, decision D-a)
    c.go(&format!("drill: post_floor {price} (keeper)"), &[post_ix(&keeper.pubkey(), &k, price, 9_000, 10)], &[&keeper])?;
    // 2. a public buy: the module's net demand
    let acc = get_associated_token_address(&h.pubkey(), &k.wx_mint);
    // 1.8 units' worth: a quarter of every intake is the reserve tier (reserve_bps 2,500), so ≥ 1 whole unit of net demand remains
    c.go("drill: mint (holder buys 1.8 units' worth)", &[ata_create(&h.pubkey(), &h.pubkey(), &k.wx_mint),
        mint_ix(&h.pubkey(), &k, &acc, price * 18 / 10)], &[&h])?;
    // 3. an advance and its return, then an advance settled by an item
    let m: Module = c.rpc.get(&k.module)?;
    let reg: Registry = c.rpc.get(&registry())?;
    let now = now_s() as i64;
    let w0: Wrapper = c.rpc.get(&k.wrapper)?;
    let e = w0.eth_backed;
    let cap = m.max_item_cost(now, e, l.keeper_down(&reg)).unwrap_or(0);
    println!("drill: max_item_cost {cap} lamports (net units {} base units)", m.net_units(e));
    let adv = cap.min(price);
    let advance = |amount: u64| ix(rook_core::instruction::Advance { amount }, rook_core::accounts::Advance {
        custodian: cust.pubkey(), registry: registry(), listing: k.listing, module: k.module, module_vault: k.vault, wrapper: k.wrapper });
    let settle_accts = rook_core::accounts::Settle { custodian: cust.pubkey(), registry: registry(), listing: k.listing, module: k.module,
        module_vault: k.vault, wrapper: k.wrapper, wx_mint: k.wx_mint, module_wx: k.module_wx, token_program: spl_token::ID };
    if adv > 0 {
        c.go(&format!("drill: advance {adv} (custodian)"), &[advance(adv)], &[&cust])?;
        c.go(&format!("drill: return_advance {adv} (custodian; the stubbed Relay leg's give-back)"), &[ix(rook_core::instruction::ReturnAdvance { lamports: adv },
            rook_core::accounts::ReturnAdvance { custodian: cust.pubkey(), listing: k.listing, module: k.module, module_vault: k.vault,
                system_program: system_program::ID })], &[&cust])?;
        c.go(&format!("drill: advance {adv} again"), &[advance(adv)], &[&cust])?;
    } else {
        c.job.step("INFO", "drill: advance", "the module states no item cost now (stale or no demand): skipped", None, None);
    }
    // 4. mint_item with a proof (the custodian's), into the module's account when an advance is open (settle_advance then burns it)
    let w: Wrapper = c.rpc.get(&k.wrapper)?;
    let mut n = w.item_count;
    let mut minted = vec![];
    for (i, id) in ids.iter().enumerate().take(2) {
        let tx = keccak(&[b"rook devnet drill", id.as_bytes(), &now_s().to_le_bytes()]);
        let dest = if i == 0 && adv > 0 { k.module_wx } else { acc };
        let r = c.go(&format!("drill: mint_item {id} with a {}-hash proof", proofs[i].len()), &[mint_item_ix(&cust.pubkey(), &k, n, frog_args(id, proofs[i].clone(), tx, i as u32), &dest)], &[&cust]);
        if r.is_ok() {
            minted.push(id.clone());
            n += 1;
        }
    }
    if adv > 0 {
        c.go(&format!("drill: settle_advance {adv} (the item backs the advanced unit)"), &[ix(rook_core::instruction::SettleAdvance { cost: adv }, settle_accts)], &[&cust])?;
    }
    // 5. redemptions: a mainnet address refused on a signet registry, a signet one taken, a burn taken
    let mainnet = reg.bitcoin_network == BitcoinNetwork::Mainnet;
    let (good, other) = if mainnet { (BC_NUMS, TB_DEST) } else { (TB_DEST, BC_NUMS) };
    let w: Wrapper = c.rpc.get(&k.wrapper)?;
    c.expect_refused(&format!("drill: redeem to {other} (the other network) is refused"),
        &[redeem_ix(&h.pubkey(), &k, &acc, w.ticket_count, Destination::Taproot(other.into()))], &[&h], &code(rook_core::error::RookError::BadDestination));
    let units = c.rpc.token_amount(&acc);
    println!("drill: the holder has {units} wX base units");
    let mut tickets = vec![];
    for (j, d) in [Destination::Taproot(good.into()), Destination::Taproot(good.into())].into_iter().enumerate() {
        let w: Wrapper = c.rpc.get(&k.wrapper)?;
        if c.rpc.token_amount(&acc) < ONE {
            c.job.step("INFO", "drill: redeem", "the holder holds under one unit: mint more (mint-units --listing drill)", None, None);
            break;
        }
        let id = w.ticket_count;
        if c.go(&format!("drill: redeem #{j} to {good} (ticket {id})"), &[redeem_ix(&h.pubkey(), &k, &acc, id, d)], &[&h]).is_ok() {
            tickets.push(id);
        }
    }
    // 6. the drand round (keeper-attested until Q4), resolve, then the custodian's records
    for id in &tickets {
        let t: Ticket = c.rpc.get(&ticket_pda(&k, *id))?;
        let rp = pda(&[DRAND, &t.round.to_le_bytes()]);
        if !c.rpc.exists(&rp) {
            let mut rnd = keccak(&[b"rook devnet randomness", &t.round.to_le_bytes()]);
            rnd[0] |= 1;
            // prove_round refuses a round still in the future (programs commit 6588b81): wait until its drand time has passed
            let at = rook_core::clock::DRAND_GENESIS + (t.round as i64 - 1) * rook_core::clock::DRAND_PERIOD + 10;
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
            if at > now {
                println!("drill: waiting {} s for drand round {} to be in the past", at - now, t.round);
                sleep(Duration::from_secs((at - now) as u64));
            }
            c.go(&format!("drill: prove_round {} (keeper)", t.round), &[prove_ix(&keeper.pubkey(), t.round, rnd)], &[&keeper])?;
        }
        c.go(&format!("drill: resolve ticket {id}"), &[resolve_ix(&k, *id, t.round)], &[&h])?;
    }
    if let Some(id) = tickets.first() {
        let txid = keccak(&[b"a signet delivery the drill never sends", &id.to_le_bytes()]);
        c.go(&format!("drill: note_sent ticket {id}"), &[custodian_ticket_ix(rook_core::instruction::NoteSent { tx: txid }, &cust.pubkey(), &k, *id)], &[&cust])?;
        let t: Ticket = c.rpc.get(&ticket_pda(&k, *id))?;
        let item = pda(&[ITEM, k.wrapper.as_ref(), &t.item_id.to_le_bytes()]);
        let it: Item = c.rpc.get(&item)?;
        c.go(&format!("drill: record_delivery ticket {id} (item {})", t.item_id), &[ix(rook_core::instruction::RecordDelivery { tx: txid },
            rook_core::accounts::RecordDelivery { custodian: cust.pubkey(), listing: k.listing, wrapper: k.wrapper, ticket: ticket_pda(&k, *id), item,
                held: pda(&[HELD, k.wrapper.as_ref(), &it.key]) })], &[&cust])?;
    }
    if let Some(id) = tickets.get(1) {
        c.go(&format!("drill: mark_undeliverable ticket {id}"), &[custodian_ticket_ix(rook_core::instruction::MarkUndeliverable {}, &cust.pubkey(), &k, *id)], &[&cust])?;
    }
    // 7. a burn of the holder's leftover units (the floor's exit)
    let left = c.rpc.token_amount(&acc);
    if left > 0 {
        let units = left.min(ONE / 10);
        c.go(&format!("drill: burn {units} base units (holder)"), &[ix(rook_core::instruction::Burn { units, min_lamports: 0 }, rook_core::accounts::BurnUnits {
            owner: h.pubkey(), registry: registry(), listing: k.listing, module: k.module, module_vault: k.vault, wrapper: k.wrapper, wx_mint: k.wx_mint,
            owner_wx: acc, to: h.pubkey(), token_program: spl_token::ID })], &[&h])?;
    }
    // 8. the invariants, read back
    let ((ok_books, books), (ok_ledger, ledger)) = invariants(&c.rpc, &k)?;
    c.job.step(if ok_books { "PASS" } else { "FAIL" }, "drill: the books", &books, None, None);
    c.job.step(if ok_ledger { "PASS" } else { "FAIL" }, "drill: the wX ledger", &ledger, None, None);
    c.job.state["drill"] = json!({ "minted": minted, "tickets": tickets });
    c.job.save();
    Ok(())
}

/// A listing's two invariants on chain: the books (the vault, less its rent, holds at least backing + reserve + surplus + revenue)
/// and the wX ledger (supply = (held − open tickets) · 10⁹ + eth_backed).
fn invariants(rpc: &Rpc, k: &Col) -> Result<((bool, String), (bool, String)), String> {
    let m: Module = rpc.get(&k.module)?;
    let (vl, _, vd) = rpc.account(&k.vault)?.ok_or("vault")?;
    let rent = rpc.call("getMinimumBalanceForRentExemption", json!([vd.len()]))?.as_u64().unwrap_or(0);
    let books = m.books();
    let free = vl.saturating_sub(rent);
    let w: Wrapper = rpc.get(&k.wrapper)?;
    let (_, _, md) = rpc.account(&k.wx_mint)?.ok_or("mint")?;
    let supply = u64::from_le_bytes(md[36..44].try_into().unwrap());
    let ledger = (w.held - w.open_tickets) * ONE + w.eth_backed;
    Ok(((free as u128 >= books, format!("vault {free} free ≥ books {books} (backing {} reserve {} surplus {} revenue {}; transit {} units)",
        m.backing, m.reserve, m.surplus, m.revenue, m.transit_units)),
        (supply == ledger, format!("supply {supply} = (held {} − open {}) · 10^9 + eth_backed {}", w.held, w.open_tickets, w.eth_backed))))
}

fn cmd_refusals(c: &mut Ctx) -> Result<(), String> {
    let o = c.payer.insecure_clone();
    let (keeper, guardian, h) = (c.role("keeper")?, c.role("guardian")?, c.role("holder")?);
    let reg: Registry = c.rpc.get(&registry())?;
    let i = ix(rook_core::instruction::InitRegistry { owner: o.pubkey(), bitcoin_network: reg.bitcoin_network, initial_converters: vec![] },
        rook_core::accounts::InitRegistry { authority: o.pubkey(), payer: o.pubkey(), registry: registry(), program_data: program_data(&rook_core::ID),
            system_program: system_program::ID });
    c.expect_refused("refusal: init_registry a second time", &[i], &[&o], "already in use");
    let other = if reg.bitcoin_network == BitcoinNetwork::Mainnet { TB_NUMS } else { BC_NUMS };
    let next = col(reg.listing_count);
    c.expect_refused(&format!("refusal: an Inscriptions listing with custody {other} (not the registry's {:?})", reg.bitcoin_network),
        &[ix(rook_core::instruction::ListCollection { args: ListArgs { kind: ListingKind::Inscriptions { membership_root: [9; 32] }, name: "x".into(),
            custody: other.into(), custodian: o.pubkey(), poster: Pubkey::default() } },
            rook_core::accounts::ListCollection { owner: o.pubkey(), payer: o.pubkey(), registry: registry(), listing: next.listing, system_program: system_program::ID })],
        &[&o], &code(rook_core::error::RookError::BadInput));
    let (_, ifr, idr) = c.ids()?;
    c.expect_refused("refusal: a floor post by the guardian on the frog listing (the keeper posts frogs)", &[post_ix(&guardian.pubkey(), &col(ifr), SOL, 9_000, 1)],
        &[&guardian], &code(rook_core::error::RookError::NotKeeper));
    let n = c.rpc.get::<Wrapper>(&col(idr).wrapper)?.item_count;
    c.expect_refused("refusal: a stranger's mint_item on the drill listing (only the listing's custodian mints)", &[mint_item_ix(&h.pubkey(), &col(idr), n,
        frog_args(SYNTH_FROGS[2], vec![], [1; 32], 0), &col(idr).module_wx)], &[&h], &code(rook_core::error::RookError::NotCustodian));
    c.expect_refused("refusal: the keeper is not the owner (set_guardian)", &[Instruction::new_with_bytes(rook_core::ID,
        &rook_core::instruction::SetGuardian { guardian: h.pubkey() }.data(), rook_core::accounts::RegistryAdmin { signer: keeper.pubkey(), registry: registry() }.to_account_metas(None))],
        &[&keeper], &code(rook_core::error::RookError::NotOwner));
    Ok(())
}

fn cmd_proof(c: &mut Ctx) -> Result<(), String> {
    let ids = frog_ids(&c.job);
    let want = c.opt("inscription").ok_or("--inscription <id>")?;
    let i = ids.iter().position(|x| *x == want).ok_or("not in this job's tree (init --frog-ids)")?;
    let (root, proofs) = tree(&ids);
    println!("{}", json!({ "inscription": want, "root": hex(&root), "proof": proofs[i].iter().map(|p| hex(p)).collect::<Vec<_>>() }));
    Ok(())
}

fn cmd_status(c: &mut Ctx) -> Result<(), String> {
    let reg: Registry = c.rpc.get(&registry())?;
    println!("registry {}: owner {} keeper {} guardian {} listings {} network {:?} converters {} pending {} (at {})", registry(), reg.owner, reg.keeper,
        reg.guardian, reg.listing_count, reg.bitcoin_network, reg.converter_count, reg.pending_converter, reg.pending_converter_at);
    if let Ok(d) = c.rpc.get::<Desk>(&pda(&[DESK])) {
        println!("desk: bond {} in [{}, {}], window {} s, requests {}", d.bond, d.min_bond, d.max_bond, d.window, d.request_count);
    }
    for id in 0..reg.listing_count {
        let k = col(id);
        let l: Listing = c.rpc.get(&k.listing)?;
        println!("listing {id} {} ({:?}) custodian {} poster {} custody {}", l.name, l.kind, l.custodian, l.poster, l.custody);
        if let (Ok(m), Ok(w)) = (c.rpc.get::<Module>(&k.module), c.rpc.get::<Wrapper>(&k.wrapper)) {
            println!("  module {}: accepted {} posted_at {} fresh {} backing {} reserve {} surplus {} revenue {} transit {} revenue_target {}", k.module, m.accepted,
                m.posted_at, m.fresh(now_s() as i64), m.backing, m.reserve, m.surplus, m.revenue, m.transit_units, m.params.revenue_target);
            println!("  wrapper {}: held {} open {} tickets {} items {} eth_backed {} (wX {})", k.wrapper, w.held, w.open_tickets, w.ticket_count, w.item_count,
                w.eth_backed, k.wx_mint);
            if let Ok(((b, bt), (l, lt))) = invariants(&c.rpc, &k) {
                println!("  books {}: {bt}; wX ledger {}: {lt}", if b { "hold" } else { "BROKEN" }, if l { "exact" } else { "BROKEN" });
            }
        }
    }
    Ok(())
}

fn cmd_accept_converter(c: &mut Ctx) -> Result<(), String> {
    let o = c.payer.insecure_clone();
    let reg: Registry = c.rpc.get(&registry())?;
    if reg.pending_converter == Pubkey::default() {
        return Err("no converter is pending".into());
    }
    if (now_s() as i64) < reg.pending_converter_at {
        return Err(format!("too early: the converter may be accepted from {} (unix), {} h from now", reg.pending_converter_at,
            (reg.pending_converter_at - now_s() as i64) / 3600));
    }
    c.go("accept_converter", &[Instruction::new_with_bytes(rook_core::ID, &rook_core::instruction::AcceptConverter {}.data(),
        rook_core::accounts::RegistryAdmin { signer: o.pubkey(), registry: registry() }.to_account_metas(None))], &[&o])?;
    Ok(())
}

// ---- Meteora DBC on this cluster (devnet has DBC, DAMM v2 and the "Customizable" DAMM config at the mainnet ids) -------------------

mod dbcx {
    use super::*;
    use rook_backing::{
        constants::*,
        dynamic_bonding_curve::{self as dbc, types as dbct},
    };
    pub const MPL_ID: Pubkey = anchor_lang::pubkey!("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s");
    pub const TOKEN_2022_ID: Pubkey = anchor_lang::pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
    pub const DAMM_CONFIG_CUSTOMIZABLE: Pubkey = anchor_lang::pubkey!("A8gMrEPJkacWkcb3DGwtJwTe16HktSEfvwtuDh2MCtck");

    pub fn p(seeds: &[&[u8]], program: &Pubkey) -> Pubkey {
        Pubkey::find_program_address(seeds, program).0
    }
    pub fn ea(program: &Pubkey) -> Pubkey {
        p(&[b"__event_authority"], program)
    }
    fn mm(a: &Pubkey, b: &Pubkey) -> (Pubkey, Pubkey) {
        if a > b { (*a, *b) } else { (*b, *a) }
    }
    pub fn pool(config: &Pubkey, base: &Pubkey) -> Pubkey {
        let (hi, lo) = mm(base, &NATIVE_MINT);
        p(&[b"pool", config.as_ref(), hi.as_ref(), lo.as_ref()], &DBC_ID)
    }
    pub fn vault(mint: &Pubkey, pool: &Pubkey) -> Pubkey {
        p(&[b"token_vault", mint.as_ref(), pool.as_ref()], &DBC_ID)
    }
    pub fn damm_pool(base: &Pubkey) -> Pubkey {
        let (hi, lo) = mm(base, &NATIVE_MINT);
        p(&[b"pool", DAMM_CONFIG_CUSTOMIZABLE.as_ref(), hi.as_ref(), lo.as_ref()], &DAMM_V2_ID)
    }
    pub fn damm_vault(mint: &Pubkey, pool: &Pubkey) -> Pubkey {
        p(&[b"token_vault", mint.as_ref(), pool.as_ref()], &DAMM_V2_ID)
    }
    pub fn settings() -> Pubkey {
        p(&[SETTINGS_SEED], &rook_backing::ID)
    }
    pub fn partner(listing: &Pubkey) -> Pubkey {
        p(&[PARTNER_SEED, listing.as_ref()], &rook_backing::ID)
    }
    /// rook_backing's program-wide ["converter"] PDA: the one key rook_core's `Registry.converters` holds for it (signs `convert`).
    pub fn converter() -> Pubkey {
        p(&[CONVERTER_SEED], &rook_backing::ID)
    }
    pub fn claimer(listing: &Pubkey) -> Pubkey {
        p(&[CLAIMER_SEED, listing.as_ref()], &rook_backing::ID)
    }
    pub fn claimer_wsol(listing: &Pubkey) -> Pubkey {
        p(&[CLAIMER_WSOL_SEED, listing.as_ref()], &rook_backing::ID)
    }
    pub fn vault_pda(coin: &Pubkey) -> Pubkey {
        p(&[VAULT_SEED, coin.as_ref()], &rook_backing::ID)
    }
    pub fn vault_wx(coin: &Pubkey) -> Pubkey {
        p(&[VAULT_WX_SEED, coin.as_ref()], &rook_backing::ID)
    }

    /// The Q1 Rook config (spike page §3) with a devnet-sized threshold and DBC's dynamic supply (the fixed 1B supply validates only for
    /// the curve it was designed for: `solana/tests/q1/design_curve.mjs 1000000000 20 <threshold> 50 6` gives that curve).
    pub fn config(threshold: u64, sqrt_start: u128, curve: &[(u128, u128)]) -> dbct::ConfigParameters {
        let zero = || dbct::LiquidityVestingInfoParams { vesting_percentage: 0, bps_per_period: 0, number_of_periods: 0, cliff_duration_from_migration_time: 0, frequency: 0 };
        dbct::ConfigParameters {
            pool_fees: dbct::PoolFeeParameters {
                base_fee: dbct::BaseFeeParameters { cliff_fee_numerator: 500_000_000, first_factor: 60, second_factor: 10, third_factor: 8_166_666, base_fee_mode: 0 },
                dynamic_fee: None,
            },
            collect_fee_mode: 0,
            migration_option: 1,
            activation_type: 1,
            token_type: 0,
            token_decimal: 6,
            partner_liquidity_percentage: 0,
            partner_permanent_locked_liquidity_percentage: 50,
            creator_liquidity_percentage: 0,
            creator_permanent_locked_liquidity_percentage: 50,
            migration_quote_threshold: threshold,
            sqrt_start_price: sqrt_start,
            locked_vesting: dbct::LockedVestingParams { amount_per_period: 0, cliff_duration_from_migration_time: 0, frequency: 0, number_of_period: 0, cliff_unlock_amount: 0 },
            migration_fee_option: 6,
            token_supply: None,
            creator_trading_fee_percentage: 0,
            token_update_authority: 1,
            migration_fee: dbct::MigrationFee { fee_percentage: 50, creator_fee_percentage: 0 },
            migrated_pool_fee: dbct::MigratedPoolFee { collect_fee_mode: 0, dynamic_fee: 0, pool_fee_bps: 100 },
            pool_creation_fee: 0,
            partner_liquidity_vesting_info: zero(),
            creator_liquidity_vesting_info: zero(),
            migrated_pool_base_fee_mode: 0,
            migrated_pool_market_cap_fee_scheduler_params: dbct::MigratedPoolMarketCapFeeSchedulerParams { number_of_period: 0, sqrt_price_step_bps: 0,
                scheduler_expiration_duration: 0, reduction_factor: 0 },
            enable_first_swap_with_min_fee: false,
            compounding_fee_bps: 0,
            padding: [0; 2],
            curve: curve.iter().map(|(s, l)| dbct::LiquidityDistributionParameters { sqrt_price: *s, liquidity: *l }).collect(),
        }
    }

    pub fn create_config(payer: &Pubkey, config: &Pubkey, claimer: &Pubkey, params: dbct::ConfigParameters) -> Instruction {
        Instruction { program_id: DBC_ID, accounts: dbc::client::accounts::CreateConfig { config: *config, fee_claimer: *claimer, leftover_receiver: *claimer,
            quote_mint: NATIVE_MINT, payer: *payer, system_program: system_program::ID, event_authority: ea(&DBC_ID), program: DBC_ID }.to_account_metas(None),
            data: dbc::client::args::CreateConfig { config_parameters: params }.data() }
    }

    pub fn launch(config: &Pubkey, creator: &Pubkey, mint: &Pubkey, payer: &Pubkey) -> Instruction {
        let pool = pool(config, mint);
        Instruction { program_id: DBC_ID, accounts: dbc::client::accounts::InitializeVirtualPoolWithSplToken { config: *config, pool_authority: DBC_POOL_AUTHORITY,
            creator: *creator, base_mint: *mint, quote_mint: NATIVE_MINT, pool, base_vault: vault(mint, &pool), quote_vault: vault(&NATIVE_MINT, &pool),
            mint_metadata: p(&[b"metadata", MPL_ID.as_ref(), mint.as_ref()], &MPL_ID), metadata_program: MPL_ID, payer: *payer,
            token_quote_program: spl_token::ID, token_program: spl_token::ID, system_program: system_program::ID, event_authority: ea(&DBC_ID), program: DBC_ID }
            .to_account_metas(None),
            data: dbc::client::args::InitializeVirtualPoolWithSplToken { params: dbct::InitializePoolParameters { name: "Rook Devnet".into(), symbol: "RKD".into(),
                uri: "https://rook.example/devnet.json".into() } }.data() }
    }

    pub fn wsol(owner: &Pubkey, lamports: u64) -> Vec<Instruction> {
        let ata = get_associated_token_address(owner, &NATIVE_MINT);
        vec![ata_create(owner, owner, &NATIVE_MINT), system_instruction::transfer(owner, &ata, lamports), spl_token::instruction::sync_native(&spl_token::ID, &ata).unwrap()]
    }

    pub fn buy(config: &Pubkey, mint: &Pubkey, buyer: &Pubkey, lamports: u64) -> Vec<Instruction> {
        let pool = pool(config, mint);
        let mut v = wsol(buyer, lamports);
        v.push(ata_create(buyer, buyer, mint));
        v.push(Instruction { program_id: DBC_ID, accounts: dbc::client::accounts::Swap2 { pool_authority: DBC_POOL_AUTHORITY, config: *config, pool,
            input_token_account: get_associated_token_address(buyer, &NATIVE_MINT), output_token_account: get_associated_token_address(buyer, mint),
            base_vault: vault(mint, &pool), quote_vault: vault(&NATIVE_MINT, &pool), base_mint: *mint, quote_mint: NATIVE_MINT, payer: *buyer,
            token_base_program: spl_token::ID, token_quote_program: spl_token::ID, referral_token_account: None, event_authority: ea(&DBC_ID), program: DBC_ID }
            .to_account_metas(None),
            data: dbc::client::args::Swap2 { params: dbct::SwapParameters2 { amount_0: lamports, amount_1: 0, swap_mode: 1 } }.data() });
        v
    }

    pub fn migrate(config: &Pubkey, mint: &Pubkey, payer: &Pubkey, nft1: &Pubkey, nft2: &Pubkey) -> Instruction {
        let pool = pool(config, mint);
        let dp = damm_pool(mint);
        let pos = |n: &Pubkey| p(&[b"position", n.as_ref()], &DAMM_V2_ID);
        let nacc = |n: &Pubkey| p(&[b"position_nft_account", n.as_ref()], &DAMM_V2_ID);
        let mut metas = dbc::client::accounts::MigrationDammV2 { virtual_pool: pool, migration_metadata: system_program::ID, config: *config,
            pool_authority: DBC_POOL_AUTHORITY, pool: dp, first_position_nft_mint: *nft1, first_position_nft_account: nacc(nft1), first_position: pos(nft1),
            second_position_nft_mint: Some(*nft2), second_position_nft_account: Some(nacc(nft2)), second_position: Some(pos(nft2)),
            damm_pool_authority: DAMM_V2_POOL_AUTHORITY, amm_program: DAMM_V2_ID, base_mint: *mint, quote_mint: NATIVE_MINT,
            token_a_vault: damm_vault(mint, &dp), token_b_vault: damm_vault(&NATIVE_MINT, &dp), base_vault: vault(mint, &pool), quote_vault: vault(&NATIVE_MINT, &pool),
            payer: *payer, token_base_program: spl_token::ID, token_quote_program: spl_token::ID, token_2022_program: TOKEN_2022_ID,
            damm_event_authority: ea(&DAMM_V2_ID), system_program: system_program::ID }.to_account_metas(None);
        metas.push(AccountMeta::new_readonly(DAMM_CONFIG_CUSTOMIZABLE, false));
        Instruction { program_id: DBC_ID, accounts: metas, data: dbc::client::args::MigrationDammV2 {}.data() }
    }

    pub fn back(k: &Col, config: &Pubkey, mint: &Pubkey, cranker: &Pubkey) -> Instruction {
        let pool = pool(config, mint);
        Instruction { program_id: rook_backing::ID, accounts: rook_backing::accounts::Back { cranker: *cranker, settings: settings(), partner: partner(&k.listing), vault: vault_pda(mint),
            dbc_pool: pool, dbc_config: *config, dbc_quote_vault: vault(&NATIVE_MINT, &pool), dbc_pool_authority: DBC_POOL_AUTHORITY, quote_mint: NATIVE_MINT,
            claimer: claimer(&k.listing), claimer_wsol: claimer_wsol(&k.listing), dbc_event_authority: ea(&DBC_ID), dbc_program: DBC_ID, rook_core: rook_core::ID,
            converter: converter(), registry: registry(), listing: k.listing, module: k.module, module_vault: k.vault, wrapper: k.wrapper, wx_mint: k.wx_mint, vault_wx: vault_wx(mint),
            coin_mint: *mint, token_program: spl_token::ID, wx_token_program: spl_token::ID, system_program: system_program::ID }.to_account_metas(None),
            data: rook_backing::instruction::Back { min_units: 0 }.data() }
    }
}

fn parse_curve(c: &Ctx) -> Result<(u64, u128, Vec<(u128, u128)>), String> {
    // default: design_curve.mjs 1000000000 20 2 50 6 (a 2 SOL threshold), run 2026-10-09
    let thr = (c.num("threshold-sol", 2.0)? * SOL as f64) as u64;
    if thr != 2 * SOL && !c.a.contains_key("curve") {
        return Err("a threshold other than 2 SOL needs --curve <design_curve.mjs JSON file> (solana/tests/q1/design_curve.mjs 1000000000 20 <sol> 50 6)".into());
    }
    if let Some(f) = c.opt("curve") {
        let j: Value = serde_json::from_str(&fs::read_to_string(&f).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let s: u128 = j["sqrtStartPrice"].as_str().unwrap_or("").parse().map_err(|_| "sqrtStartPrice")?;
        let pts = j["curve"].as_array().ok_or("curve")?.iter()
            .map(|p| Ok((p["sqrtPrice"].as_str().unwrap_or("").parse().map_err(|_| "sqrtPrice")?, p["liquidity"].as_str().unwrap_or("").parse().map_err(|_| "liquidity")?)))
            .collect::<Result<Vec<_>, String>>()?;
        let t: u64 = j["migrationQuoteThreshold"].as_str().unwrap_or("").parse().map_err(|_| "migrationQuoteThreshold")?;
        return Ok((t, s, pts));
    }
    Ok((thr, 20624089540570002, vec![(41248173712355948, 32998543264912563173317949192330), (79226673521066979257578248091, 3436021254348803974616125)]))
}

fn cmd_dbc_launch(c: &mut Ctx) -> Result<(), String> {
    use rook_backing::constants::DBC_ID;
    let o = c.payer.insecure_clone();
    let k = col(c.ids()?.0);
    if !c.rpc.exists(&DBC_ID) {
        return Err("Meteora DBC is not deployed on this cluster (devnet has it at the mainnet id; a local validator needs --clone)".into());
    }
    let h = c.role("holder")?;
    // init_backing (the upgrade authority) and the partner of listing 0 (the registry owner)
    let settings = dbcx::settings();
    c.once("rook_backing init_backing", c.rpc.exists(&settings), &[Instruction { program_id: rook_backing::ID,
        accounts: rook_backing::accounts::InitBacking { authority: o.pubkey(), settings, program: rook_backing::ID, program_data: program_data(&rook_backing::ID),
            system_program: system_program::ID }.to_account_metas(None), data: rook_backing::instruction::InitBacking {}.data() }], &[&o])?;
    let partner = dbcx::partner(&k.listing);
    let params = rook_backing::PartnerParams { listing: k.listing, registry: registry(), module: k.module, module_vault: k.vault, wrapper: k.wrapper, wx_mint: k.wx_mint };
    c.once(&format!("rook_backing register_partner (listing {})", k.id), c.rpc.exists(&partner), &[Instruction { program_id: rook_backing::ID,
        accounts: rook_backing::accounts::RegisterPartner { admin: o.pubkey(), settings, registry: registry(), partner, listing: k.listing, module: k.module,
            module_vault: k.vault, wrapper: k.wrapper, system_program: system_program::ID }
            .to_account_metas(None), data: rook_backing::instruction::RegisterPartner { params }.data() }], &[&o])?;
    let (thr, sqrt_start, curve) = parse_curve(c)?;
    let claimer = dbcx::claimer(&k.listing);
    let cfg = Keypair::new();
    c.go(&format!("dbc create_config (threshold {thr} lamports, 50 % migration fee to the claimer PDA {claimer})"),
        &[dbcx::create_config(&o.pubkey(), &cfg.pubkey(), &claimer, dbcx::config(thr, sqrt_start, &curve))], &[&o, &cfg])?;
    c.go("rook_backing register_partner_config", &[Instruction { program_id: rook_backing::ID, accounts: rook_backing::accounts::RegisterPartnerConfig {
        admin: o.pubkey(), settings, registry: registry(), partner, dbc_config: cfg.pubkey() }.to_account_metas(None),
        data: rook_backing::instruction::RegisterPartnerConfig {}.data() }], &[&o])?;
    let mint = Keypair::new();
    c.go("dbc initialize_virtual_pool_with_spl_token (a coin launched; creator = the holder)", &[dbcx::launch(&cfg.pubkey(), &h.pubkey(), &mint.pubkey(), &o.pubkey())],
        &[&o, &h, &mint])?;
    let pool = dbcx::pool(&cfg.pubkey(), &mint.pubkey());
    c.go("rook_backing open_vault", &[Instruction { program_id: rook_backing::ID, accounts: rook_backing::accounts::OpenVault { payer: o.pubkey(), partner,
        dbc_pool: pool, coin_mint: mint.pubkey(), wx_mint: k.wx_mint, vault: dbcx::vault_pda(&mint.pubkey()), vault_wx: dbcx::vault_wx(&mint.pubkey()),
        wx_token_program: spl_token::ID, system_program: system_program::ID }.to_account_metas(None), data: rook_backing::instruction::OpenVault {}.data() }], &[&o])?;
    c.job.state["dbc"] = json!({ "config": cfg.pubkey().to_string(), "coin": mint.pubkey().to_string(), "pool": pool.to_string(), "threshold": thr });
    c.job.save();
    if !c.a.contains_key("no-wait") {
        println!("waiting 610 s for the anti-sniper fee to decay (50 % → 1 %; --no-wait pays it)");
        sleep(Duration::from_secs(610));
    }
    // after the anti-sniper window ≈ 1 % fee: 1.5× the threshold is plenty (PartialFill stops at it); at launch the fee is 50 %: 3×
    let spend = thr * if c.a.contains_key("no-wait") { 3 } else { 3 } / if c.a.contains_key("no-wait") { 1 } else { 2 };
    c.go(&format!("dbc swap2 PartialFill {spend} lamports (the holder buys to the threshold)"), &dbcx::buy(&cfg.pubkey(), &mint.pubkey(), &h.pubkey(), spend), &[&h])?;
    let (n1, n2) = (Keypair::new(), Keypair::new());
    c.go("dbc migration_damm_v2 (permissionless; the payer cranks)", &[dbcx::migrate(&cfg.pubkey(), &mint.pubkey(), &o.pubkey(), &n1.pubkey(), &n2.pubkey())],
        &[&o, &n1, &n2])?;
    // back → convert in the same run: rook_backing's ["converter"] PDA is allowed from init_registry
    let reg: Registry = c.rpc.get(&registry())?;
    if reg.is_converter(&dbcx::converter()) {
        return dbc_back(c, &k, &cfg.pubkey(), &mint.pubkey());
    }
    {
        // a registry initialized before initial converters (the converter proposed, not yet accepted): simulated, DBC's
        // withdraw_migration_fee runs (rook_backing signing as the claimer PDA), then rook_core's convert refuses the converter PDA
        // until the owner accepts it (7 days): the whole transaction reverts, nothing moves; `accept-converter` then `dbc-back`
        let tx = build(&c.rpc, &[dbcx::back(&k, &cfg.pubkey(), &mint.pubkey(), &o.pubkey())], &[&o])?;
        let v = c.rpc.call("simulateTransaction", json!([b64(&tx), { "encoding": "base64", "commitment": "confirmed", "sigVerify": false }]))?;
        let logs: Vec<String> = v["value"]["logs"].as_array().map(|a| a.iter().filter_map(|l| l.as_str().map(String::from)).collect()).unwrap_or_default();
        let dbc_ok = logs.iter().any(|l| l.starts_with(&format!("Program {} success", rook_backing::constants::DBC_ID)));
        let refused = v["value"]["err"].to_string().contains(&code(rook_core::error::RookError::NotConverter));
        c.job.step(if dbc_ok && refused { "PASS" } else { "FAIL" }, "dbc: back before the converter is accepted (a registry from before initial converters)",
            &format!("DBC withdraw_migration_fee CPI from rook_backing {}; rook_core convert {} ({})", if dbc_ok { "ran" } else { "did NOT run" },
                if refused { "refused NotConverter" } else { "not refused as expected" }, v["value"]["err"]), None, v["value"]["unitsConsumed"].as_u64());
    }
    Ok(())
}

fn cmd_dbc_back(c: &mut Ctx) -> Result<(), String> {
    let k = col(c.ids()?.0);
    let d = c.job.get("dbc").cloned().ok_or("run dbc-launch first")?;
    let cfg: Pubkey = d["config"].as_str().unwrap().parse().unwrap();
    let mint: Pubkey = d["coin"].as_str().unwrap().parse().unwrap();
    dbc_back(c, &k, &cfg, &mint)
}

/// `back` (DBC withdraw_migration_fee → rook_core convert signed by the ["converter"] PDA, paid by the claimer), then a holder redeems.
fn dbc_back(c: &mut Ctx, k: &Col, cfg: &Pubkey, mint: &Pubkey) -> Result<(), String> {
    let (k, cfg, mint) = (*k, *cfg, *mint);
    let o = c.payer.insecure_clone();
    // `back` runs once per coin (the migration fee is withdrawn once): a vault already backed is not backed again (a rerun after a
    // `back` that landed while the tool lost sight of it: run 3, 2026-10-09)
    let before: rook_backing::Vault = c.rpc.get(&dbcx::vault_pda(&mint))?;
    if before.wx_units > 0 {
        c.job.step("PASS", "rook_backing back: already landed earlier", &format!("the vault was backed at {} with {} lamports → {} wX base units",
            before.backed_at, before.lamports_backed, before.wx_units), None, None);
    } else {
        back_now(c, &k, &cfg, &mint, &o)?;
    }
    let v: rook_backing::Vault = c.rpc.get(&dbcx::vault_pda(&mint))?;
    println!("vault: {} wX base units for the coin {mint}", v.wx_units);
    redeem_tenth(c, &k, &mint)
}

fn back_now(c: &mut Ctx, k: &Col, cfg: &Pubkey, mint: &Pubkey, o: &Keypair) -> Result<(), String> {
    let (k, cfg, mint) = (*k, *cfg, *mint);
    // convert prices at the listing's floor: it must be fresh (the eth listing's poster is the keeper key; 8 SOL a unit, as drive.py's default)
    let m: Module = c.rpc.get(&k.module)?;
    if !m.fresh(now_s() as i64) {
        let keeper = c.role("keeper")?;
        c.go(&format!("post_floor listing {} (8 SOL a unit, the keeper key: convert needs a fresh floor)", k.id),
            &[post_ix(&keeper.pubkey(), &k, 8 * SOL, 9000, 10)], &[&keeper])?;
    }
    c.go("rook_backing back (withdraw_migration_fee → rook_core convert, signed by the [\"converter\"] PDA)", &[dbcx::back(&k, &cfg, &mint, &o.pubkey())], &[o])?;
    Ok(())
}

fn redeem_tenth(c: &mut Ctx, k: &Col, mint: &Pubkey) -> Result<(), String> {
    let (k, mint) = (*k, *mint);
    // a holder burns a tenth of its coins for its pro-rata share of the vault's wX
    let h = c.role("holder")?;
    let holder_coin = get_associated_token_address(&h.pubkey(), &mint);
    let amount = c.rpc.token_amount(&holder_coin) / 10;
    let holder_wx = get_associated_token_address(&h.pubkey(), &k.wx_mint);
    c.go(&format!("rook_backing redeem_backing ({amount} coin base units for their share of the vault's wX)"), &[ata_create(&h.pubkey(), &h.pubkey(), &k.wx_mint),
        Instruction { program_id: rook_backing::ID, accounts: rook_backing::accounts::RedeemBacking { holder: h.pubkey(), vault: dbcx::vault_pda(&mint),
            coin_mint: mint, holder_coin, vault_wx: dbcx::vault_wx(&mint), wx_mint: k.wx_mint, holder_wx, coin_token_program: spl_token::ID,
            wx_token_program: spl_token::ID }.to_account_metas(None), data: rook_backing::instruction::RedeemBacking { amount }.data() }], &[&h])?;
    println!("the holder holds {} wX base units of listing {}", c.rpc.token_amount(&holder_wx), k.id);
    Ok(())
}

fn cmd_report(c: &mut Ctx) -> Result<(), String> {
    let steps = c.job.state["steps"].as_array().cloned().unwrap_or_default();
    let (mut p, mut f) = (0, 0);
    for s in &steps {
        match s["kind"].as_str() {
            Some("PASS") => p += 1,
            Some("FAIL") => f += 1,
            _ => {}
        }
        println!("{} {}: {}{}", s["kind"].as_str().unwrap_or(""), s["name"].as_str().unwrap_or(""), s["detail"].as_str().unwrap_or(""),
            s["cu"].as_u64().map(|x| format!(" [{x} CU]")).unwrap_or_default());
    }
    println!("{p} PASS, {f} FAIL");
    if f > 0 { Err(format!("{f} step(s) failed")) } else { Ok(()) }
}

fn main() {
    let a = match args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2)
        }
    };
    let run = || -> Result<(), String> {
        let rpc = Rpc { url: a.rpc.clone() };
        let cluster = check_cluster(&rpc)?;
        let payer = read_kp(&a.payer)?;
        let mut c = Ctx { rpc, job: Job::open(&a.job)?, payer, a: a.rest.clone() };
        c.job.state["rpc"] = json!(a.rpc);
        c.job.state["cluster"] = json!(cluster);
        c.job.save();
        if !c.rpc.exists(&rook_core::ID) && !["keys", "report", "pay", "balance", "plan"].contains(&a.cmd.as_str()) {
            return Err(format!("rook_core ({}) is not deployed on this cluster", rook_core::ID));
        }
        match a.cmd.as_str() {
            "keys" => cmd_keys(&mut c),
            "fund" => cmd_fund(&mut c),
            "plan" => cmd_plan(&mut c),
            "init" => cmd_init(&mut c),
            "post" => cmd_post(&mut c),
            "mint-units" => cmd_mint_units(&mut c),
            "fund-reserve" => cmd_fund_reserve(&mut c),
            "burn-units" => cmd_burn_units(&mut c),
            "pay" => cmd_pay(&mut c),
            "balance" => cmd_balance(&mut c),
            "drill" => cmd_drill(&mut c),
            "refusals" => cmd_refusals(&mut c),
            "proof" => cmd_proof(&mut c),
            "status" => cmd_status(&mut c),
            "accept-converter" => cmd_accept_converter(&mut c),
            "dbc-launch" => cmd_dbc_launch(&mut c),
            "dbc-back" => cmd_dbc_back(&mut c),
            "report" => cmd_report(&mut c),
            other => Err(format!("unknown command {other}")),
        }
    };
    if let Err(e) = run() {
        eprintln!("FAIL {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tree_proves_every_leaf_against_rook_cores_verify() {
        for n in 1..9 {
            let ids: Vec<String> = (0..n).map(|i| format!("{i:064x}i{i}")).collect();
            let (root, proofs) = tree(&ids);
            for (i, id) in ids.iter().enumerate() {
                assert!(rook_core::hashing::verify_proof(&proofs[i], &root, leaf_of(id)), "{n} leaves, leaf {i}");
            }
        }
    }

    #[test]
    fn the_wire_format_is_the_runtimes() {
        let k = Keypair::new();
        let msg = Message::new_with_blockhash(&[system_instruction::transfer(&k.pubkey(), &k.pubkey(), 1)], Some(&k.pubkey()), &Default::default());
        let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &[&k]).unwrap();
        let b = bincode_tx(&tx);
        assert_eq!(b[0], 1);
        assert_eq!(&b[1..65], tx.signatures[0].as_ref());
        assert_eq!(shortvec(300), vec![0xac, 0x02]);
    }

    #[test]
    fn the_signet_constants_are_taproot_on_tb() {
        assert!(rook_core::hashing::is_taproot_on(TB_NUMS, "tb"));
        assert!(rook_core::hashing::is_taproot_on(TB_DEST, "tb"));
        assert!(rook_core::hashing::is_taproot_on(BC_NUMS, "bc"));
    }
}
