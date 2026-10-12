//! Rook sim, Bitcoin half (ops/rehearsal/sim/README.md). The canister's OWN frog flow — `frogflow::tick_full`, which runs
//! `frogbuy::judge`, `sighash::taproot_key_path`, `psbt::finalize`, the depth and custody checks — compiled natively and driven
//! against a REAL regtest bitcoind and a REAL ord server. What is a stand-in is said where it is:
//!
//! - signing: a local BIP340 key with the BIP341/BIP86 tweak stands in for the subnet's threshold Schnorr key (same digest in,
//!   same 64-byte key-path signature out);
//! - EVM / 4663: not simulated (the operator's scope: Rook moves to Solana). The few reads the tick makes of the wrapper and the
//!   listing registry are answered by a stub (we are the custodian; the collection root is the sim's members root); every EVM
//!   write errors. The frog is a record created `Wanted` directly, as the floor trigger would have made it;
//! - BTC arriving at the float: funded directly on regtest (stands in for Relay's BTC leg);
//! - the runner: this program hands the marketplace PSBT to the flow exactly as `frog_offer` stores it (an `Offer`);
//! - the marketplace + seller: ops/rehearsal/sim/market.mjs (the seller's own wallet signs SIGHASH_SINGLE|ANYONECANPAY).
//!
//! Nothing here touches a public network: every URL is 127.0.0.1, every key is generated for regtest.

use std::collections::HashMap;
use std::future::Future;
use std::pin::pin;
use std::process::Command;
use std::task::{Context, Poll, Waker};

use etch_autobuyer::btcnet::BtcSeen;
use etch_autobuyer::custody::{Contents, Location, Outpoint, Output, Utxo};
use etch_autobuyer::deposits::{self, InView, OutView, TxView};
use etch_autobuyer::floortrigger::FloorState;
use etch_autobuyer::frogbuy::{self, FPhase, FrogRec};
use etch_autobuyer::frogflow::{self, FrogIo, FrogState, Offer};
use etch_autobuyer::handin::HandinState;
use etch_autobuyer::itemsink::Clock;
use etch_autobuyer::looper::EvmView;
use etch_autobuyer::{custodycap, outbound, outflow, policy};
use k256::elliptic_curve::ops::Reduce;
use k256::elliptic_curve::sec1::ToEncodedPoint;
use k256::{Scalar, U256};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const E: u128 = 1_000_000_000_000_000_000;
const W: [u8; 20] = [0xaa; 20];
const REG: [u8; 20] = [0xab; 20];
const ME: [u8; 20] = [0xcc; 20];
/// The reference rate, sats per ETH (a stand-in for the oracle; only the cost bookkeeping reads it).
const RATE: u64 = 3_190_000;

fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

fn sha256(b: &[u8]) -> [u8; 32] {
    Sha256::digest(b).into()
}
fn tagged(tag: &str, msg: &[u8]) -> [u8; 32] {
    let t = sha256(tag.as_bytes());
    sha256(&[&t[..], &t[..], msg].concat())
}

// ---- keys: the stand-in for the subnet's threshold Schnorr key ------------------------------------------------------------------

struct TapKey {
    internal: [u8; 32],
    output: [u8; 32],
    tweaked: k256::schnorr::SigningKey,
}

impl TapKey {
    /// A BIP86 key-path key: internal P (even y), output Q = P + H_TapTweak(P)·G, signing with the tweaked secret.
    fn from_seed(seed: &str) -> Self {
        let sk = k256::SecretKey::from_slice(&sha256(seed.as_bytes())).expect("a key");
        let enc = sk.public_key().to_encoded_point(true);
        let odd = enc.as_bytes()[0] == 3;
        let internal: [u8; 32] = enc.as_bytes()[1..33].try_into().unwrap();
        let mut d: Scalar = *sk.to_nonzero_scalar();
        if odd {
            d = -d;
        }
        let t = <Scalar as Reduce<U256>>::reduce_bytes(&tagged("TapTweak", &internal).into());
        let tweaked = k256::schnorr::SigningKey::from_bytes(&(d + t).to_bytes()).expect("a tweaked key");
        let output: [u8; 32] = tweaked.verifying_key().to_bytes().into();
        TapKey { internal, output, tweaked }
    }
    fn script(&self) -> Vec<u8> {
        [vec![0x51, 0x20], self.output.to_vec()].concat()
    }
    fn sign(&self, h: [u8; 32]) -> Result<Vec<u8>, String> {
        let aux = sha256(&[&h[..], b"rook-sim-aux"].concat());
        let s = self.tweaked.sign_raw(&h, &aux).map_err(|e| format!("sign: {e}"))?;
        Ok(s.to_bytes().to_vec())
    }
}

// ---- the services, over curl (no HTTP crate) ------------------------------------------------------------------------------------

struct Env {
    rpc: String,
    user: String,
    pass: String,
    ord: String,
}

impl Env {
    fn rpc_at(&self, wallet: Option<&str>, method: &str, params: Value) -> Result<Value, String> {
        let url = match wallet { Some(w) => format!("{}/wallet/{w}", self.rpc), None => self.rpc.clone() };
        let body = json!({"jsonrpc": "1.0", "id": "sim", "method": method, "params": params}).to_string();
        let out = Command::new("curl").args(["-s", "--user", &format!("{}:{}", self.user, self.pass), "-H", "content-type: application/json",
            "--data-binary", &body, &url]).output().map_err(|e| format!("curl: {e}"))?;
        let v: Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("{method}: not JSON: {e}: {}", String::from_utf8_lossy(&out.stdout)))?;
        if !v["error"].is_null() {
            return Err(format!("{method}: {}", v["error"]));
        }
        Ok(v["result"].clone())
    }
    fn rpc(&self, method: &str, params: Value) -> Result<Value, String> {
        self.rpc_at(None, method, params)
    }
    /// GET an ord JSON page: (status, body).
    fn ord_get(&self, path: &str) -> Result<(u16, Value), String> {
        let out = Command::new("curl").args(["-s", "-H", "Accept: application/json", "-w", "\n%{http_code}", &format!("{}{path}", self.ord)])
            .output().map_err(|e| format!("curl: {e}"))?;
        let s = String::from_utf8_lossy(&out.stdout).to_string();
        let (body, code) = s.rsplit_once('\n').ok_or("ord: no status")?;
        let code: u16 = code.trim().parse().map_err(|_| "ord: bad status")?;
        Ok((code, serde_json::from_str(body).unwrap_or(Value::Null)))
    }
    fn mine(&self, n: u64, to: &str) -> Result<(), String> {
        self.rpc("generatetoaddress", json!([n, to])).map(|_| ())
    }
    /// Wait for ord to index up to bitcoind's tip.
    fn ord_synced(&self) -> Result<(), String> {
        let tip = self.rpc("getblockcount", json!([]))?.as_u64().unwrap_or(0);
        for _ in 0..120 {
            let (c, v) = self.ord_get("/blockheight")?;
            if c == 200 && v.as_u64() == Some(tip) {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        Err("ord did not catch up with bitcoind".into())
    }
}

fn sats(btc: &Value) -> u64 {
    (btc.as_f64().unwrap_or(0.0) * 1e8).round() as u64
}
fn h32(s: &str) -> Result<[u8; 32], String> {
    hex::decode(s).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| format!("{s:?} is not 32 bytes of hex"))
}
fn word(v: u128) -> Vec<u8> {
    let mut w = vec![0u8; 16];
    w.extend(v.to_be_bytes());
    w
}
fn aword(a: &[u8; 20]) -> Vec<u8> {
    let mut w = vec![0u8; 12];
    w.extend(a);
    w
}

// ---- the world the canister's flow sees ---------------------------------------------------------------------------------------

struct LiveIo {
    env: Env,
    float: TapKey,
    custody: TapKey,
    root: [u8; 32],
    offers: HashMap<u64, Offer>,
    saved: Option<FrogState>,
    caps: custodycap::CapPolicy,
    book: custodycap::Book,
    /// every raw transaction the flow broadcast, and every digest it signed
    sent: Vec<String>,
    signed: Vec<[u8; 32]>,
}

impl LiveIo {
    fn raw_tx(&self, txid: &[u8; 32]) -> Result<Value, String> {
        self.env.rpc("getrawtransaction", json!([hex::encode(txid), true]))
    }
    fn scan(&self, script: &[u8]) -> Result<Vec<Utxo>, String> {
        let r = self.env.rpc("scantxoutset", json!(["start", [format!("raw({})", hex::encode(script))]]))?;
        let mut v: Vec<Utxo> = r["unspents"].as_array().cloned().unwrap_or_default().iter().map(|u| Ok(Utxo {
            outpoint: Outpoint { txid: h32(u["txid"].as_str().unwrap_or(""))?, vout: u["vout"].as_u64().unwrap_or(0) as u32 },
            value: sats(&u["amount"]), confirmed: true })).collect::<Result<_, String>>()?;
        v.sort_by(|a, b| (a.outpoint.txid, a.outpoint.vout).cmp(&(b.outpoint.txid, b.outpoint.vout)));
        Ok(v)
    }
}

impl FrogIo for LiveIo {
    fn now(&self) -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
    }
    fn chain_id(&self) -> u64 {
        4663
    }
    fn wrapper(&self) -> [u8; 20] {
        W
    }
    fn fresh(&mut self) {}
    // STUB (EVM not simulated): the wrapper names us custodian; the registry's listing carries the sim's members root
    async fn eth_read(&mut self, to: [u8; 20], data: &[u8]) -> Result<Vec<u8>, String> {
        let sel: [u8; 4] = data[..4].try_into().map_err(|_| "short call")?;
        Ok(match (to, sel) {
            (W, deposits::CUSTODIAN) => aword(&ME),
            (W, deposits::REGISTRY) => aword(&REG),
            (W, deposits::LISTING_ID) => word(0),
            (W, deposits::DEPOSIT_MINTED) | (W, deposits::INSCRIPTION_HELD) => word(0),
            (REG, deposits::LISTING) => [word(32), word(1), word(0), word(0), word(0), aword(&[9; 20]), self.root.to_vec()].concat(),
            _ => return Err(format!("EVM not simulated: read 0x{} {}", hex::encode(to), hex::encode(sel))),
        })
    }
    async fn clock(&mut self) -> Result<Clock, String> {
        Ok(Clock { time: self.now(), parent_block: 0 })
    }
    async fn our_evm(&mut self) -> Result<[u8; 20], String> {
        Ok(ME)
    }
    async fn evm_nonce(&mut self) -> Result<u64, String> {
        Err("EVM not simulated".into())
    }
    async fn base_fee(&mut self) -> Result<u128, String> {
        Err("EVM not simulated".into())
    }
    async fn estimate_gas(&mut self, _to: [u8; 20], _value: u128, _data: &[u8]) -> Result<u64, String> {
        Err("EVM not simulated".into())
    }
    async fn sign_evm(&mut self, _hash: [u8; 32]) -> Result<(u8, [u8; 32], [u8; 32]), String> {
        Err("EVM not simulated".into())
    }
    async fn send_evm(&mut self, _raw: &[u8]) -> Result<(), String> {
        Err("EVM not simulated".into())
    }
    async fn evm_seen(&mut self, _hash: [u8; 32]) -> EvmView {
        EvmView::Unsure("EVM not simulated".into())
    }
    async fn float_script(&mut self) -> Result<Vec<u8>, String> {
        Ok(self.float.script())
    }
    async fn custody_script(&mut self) -> Result<Vec<u8>, String> {
        Ok(self.custody.script())
    }
    // REAL: ord's /inscription/<id>
    async fn locate(&mut self, inscription: &str) -> Result<Option<Location>, String> {
        let (c, v) = self.env.ord_get(&format!("/inscription/{inscription}"))?;
        if c == 404 {
            return Ok(None);
        }
        if c != 200 {
            return Err(format!("ord: HTTP {c}"));
        }
        let sp = v["satpoint"].as_str().ok_or("ord: no satpoint")?;
        let mut it = sp.split(':');
        let (t, o, off) = (it.next().unwrap_or(""), it.next().unwrap_or(""), it.next().unwrap_or(""));
        Ok(Some(Location { output: Outpoint { txid: h32(t)?, vout: o.parse().map_err(|_| "vout")? }, offset: off.parse().map_err(|_| "offset")?,
            value: v["value"].as_u64().ok_or("ord: no value")? }))
    }
    // REAL: ord's /output/<outpoint>
    async fn contents(&mut self, o: &Outpoint) -> Result<Option<Contents>, String> {
        let (c, v) = self.env.ord_get(&format!("/output/{}:{}", hex::encode(o.txid), o.vout))?;
        if c == 404 {
            return Ok(None);
        }
        if c != 200 || v["indexed"] == json!(false) {
            return Ok(None);
        }
        let ins = v["inscriptions"].as_array().cloned().unwrap_or_default().iter().filter_map(|x| x.as_str().map(String::from)).collect();
        let runes = v["runes"].as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
        Ok(Some(Contents { inscriptions: ins, runes, value: v["value"].as_u64().unwrap_or(0) }))
    }
    // REAL: bitcoind (txindex) and gettxout
    async fn output(&mut self, o: &Outpoint) -> Result<(Output, bool), String> {
        let t = self.raw_tx(&o.txid)?;
        let out = &t["vout"][o.vout as usize];
        if out.is_null() {
            return Err(format!("no output {}:{}", hex::encode(o.txid), o.vout));
        }
        let script = hex::decode(out["scriptPubKey"]["hex"].as_str().unwrap_or("")).map_err(|e| e.to_string())?;
        let confirmed = t["confirmations"].as_u64().unwrap_or(0) > 0;
        let unspent = self.env.rpc("gettxout", json!([hex::encode(o.txid), o.vout, true]))?;
        Ok((Output { script, value: sats(&out["value"]), confirmed }, unspent.is_null()))
    }
    async fn outspend(&mut self, o: &Outpoint) -> Result<Option<([u8; 32], bool)>, String> {
        if !self.env.rpc("gettxout", json!([hex::encode(o.txid), o.vout, true]))?.is_null() {
            return Ok(None);
        }
        let hit = |tx: &Value| tx["vin"].as_array().is_some_and(|ins| ins.iter().any(|i|
            i["txid"].as_str() == Some(&hex::encode(o.txid)) && i["vout"].as_u64() == Some(o.vout as u64)));
        for t in self.env.rpc("getrawmempool", json!([]))?.as_array().cloned().unwrap_or_default() {
            let tx = self.env.rpc("getrawtransaction", json!([t, true]))?;
            if hit(&tx) {
                return Ok(Some((h32(t.as_str().unwrap_or(""))?, false)));
            }
        }
        let tip = self.env.rpc("getblockcount", json!([]))?.as_u64().unwrap_or(0);
        for h in (0..=tip).rev() {
            let bh = self.env.rpc("getblockhash", json!([h]))?;
            let b = self.env.rpc("getblock", json!([bh, 2]))?;
            for tx in b["tx"].as_array().cloned().unwrap_or_default() {
                if hit(&tx) {
                    return Ok(Some((h32(tx["txid"].as_str().unwrap_or(""))?, true)));
                }
            }
        }
        Err("spent, but no spender found".into())
    }
    async fn tx_status(&mut self, txid: [u8; 32]) -> BtcSeen {
        let Ok(t) = self.raw_tx(&txid) else { return BtcSeen::NotYet };
        match t["blockhash"].as_str() {
            None => BtcSeen::Mempool,
            Some(b) => match self.env.rpc("getblockheader", json!([b])) {
                Ok(h) => BtcSeen::Confirmed { height: h["height"].as_u64().unwrap_or(0), block_hash: b.to_string() },
                Err(e) => BtcSeen::Unsure(e),
            },
        }
    }
    async fn tip(&mut self) -> Result<u64, String> {
        self.env.rpc("getblockcount", json!([]))?.as_u64().ok_or("no count".into())
    }
    async fn block_hash(&mut self, height: u64) -> Result<String, String> {
        self.env.rpc("getblockhash", json!([height]))?.as_str().map(String::from).ok_or("no hash".into())
    }
    // REAL: bitcoind's sendrawtransaction (full consensus and policy checks, the seller's 0x83 signature and ours verified)
    async fn btc_send(&mut self, raw: &[u8]) -> Result<(), String> {
        let r = self.env.rpc("sendrawtransaction", json!([hex::encode(raw)]));
        match &r {
            Ok(t) => println!("    [bitcoind] sendrawtransaction accepted: {}", t.as_str().unwrap_or("")),
            Err(e) => println!("    [bitcoind] sendrawtransaction REFUSED: {e}"),
        }
        self.sent.push(hex::encode(raw));
        r.map(|_| ())
    }
    async fn custody_utxos(&mut self) -> Result<Vec<Utxo>, String> {
        self.scan(&self.custody.script())
    }
    async fn tx_view(&mut self, txid: [u8; 32]) -> Result<TxView, String> {
        let t = self.raw_tx(&txid)?;
        let mut inputs = vec![];
        for i in t["vin"].as_array().cloned().unwrap_or_default() {
            let prev = Outpoint { txid: h32(i["txid"].as_str().unwrap_or(""))?, vout: i["vout"].as_u64().unwrap_or(0) as u32 };
            let pt = self.raw_tx(&prev.txid)?;
            inputs.push(InView { prev, prev_script: hex::decode(pt["vout"][prev.vout as usize]["scriptPubKey"]["hex"].as_str().unwrap_or("")).unwrap_or_default(),
                script_sig: hex::decode(i["scriptSig"]["hex"].as_str().unwrap_or("")).unwrap_or_default(),
                witness: i["txinwitness"].as_array().cloned().unwrap_or_default().iter().map(|w| hex::decode(w.as_str().unwrap_or("")).unwrap_or_default()).collect() });
        }
        let outputs = t["vout"].as_array().cloned().unwrap_or_default().iter().map(|o| OutView { value: sats(&o["value"]),
            script: hex::decode(o["scriptPubKey"]["hex"].as_str().unwrap_or("")).unwrap_or_default() }).collect();
        let block_hash = t["blockhash"].as_str().map(String::from);
        let height = match &block_hash { Some(b) => self.env.rpc("getblockheader", json!([b]))?["height"].as_u64(), None => None };
        Ok(TxView { txid, inputs, outputs, confirmed: block_hash.is_some(), height, block_hash })
    }
    async fn fee_rate(&mut self) -> Result<u64, String> {
        Ok(2)
    }
    async fn rate(&mut self) -> Result<u64, String> {
        Ok(RATE)
    }
    async fn input_clean(&mut self, o: Outpoint) -> Result<(), String> {
        match self.contents(&o).await? {
            Some(c) if c.inscriptions.is_empty() && c.runes.is_empty() => Ok(()),
            Some(c) => Err(format!("refused: {}:{} carries {:?} {:?}", hex::encode(o.txid), o.vout, c.inscriptions, c.runes)),
            None => Err("ord has not indexed it".into()),
        }
    }
    // STAND-IN for sign_with_schnorr (BIP341 aux, empty merkle root): the same digest, a real BIP340 signature
    async fn sign_float(&mut self, sighash: [u8; 32]) -> Result<Vec<u8>, String> {
        self.signed.push(sighash);
        self.float.sign(sighash)
    }
    async fn sign_custody(&mut self, sighash: [u8; 32]) -> Result<Vec<u8>, String> {
        self.custody.sign(sighash)
    }
    fn take_offer(&mut self, frog: u64) -> Option<Offer> {
        self.offers.remove(&frog)
    }
    fn save(&mut self, st: &FrogState) {
        self.saved = Some(st.clone());
    }
    fn btc_hrp(&self) -> Result<&'static str, String> {
        Ok("bcrt")
    }
    fn with_caps<R>(&mut self, f: impl FnOnce(&custodycap::CapPolicy, &mut custodycap::Book) -> R) -> R {
        f(&self.caps, &mut self.book)
    }
    fn outflow_check(&mut self, _key: &str, _collection: &str, _flow: outflow::Flow, _amount: u128) -> Result<(), String> {
        Ok(())
    }
    fn outflow_record(&mut self, _key: &str, _collection: &str, _flow: outflow::Flow, _amount: u128) {}
    fn outbound_policy(&self, custody_script: &[u8]) -> outbound::BtcPolicy {
        outbound::BtcPolicy { custody_script: custody_script.to_vec(), max_fee_sats: outbound::MAX_FEE_SATS, require_finalized: true,
            require_event: true }
    }
    fn paused(&self) -> bool {
        false
    }
    async fn float_utxos(&mut self) -> Result<Vec<Utxo>, String> {
        self.scan(&self.float.script())
    }
    async fn float_address(&mut self) -> Result<String, String> {
        Ok(policy::taproot_address(&self.float.output, "bcrt"))
    }
    async fn float_internal_key(&mut self) -> Result<[u8; 32], String> {
        Ok(self.float.internal)
    }
}

// ---- the run ------------------------------------------------------------------------------------------------------------------

fn tick(io: &mut LiveIo, st: &mut FrogState, label: &str) -> Vec<String> {
    println!("  tick ({label}):");
    let lines = block_on(frogflow::tick_full(io, st, &mut FloorState::default(), &mut HandinState::default(), false, &[]));
    for l in &lines {
        println!("    | {l}");
    }
    let f = st.frogs.iter().find(|f| f.id == 0).unwrap();
    println!("    => frog 0 phase: {}", phase_name(&f.phase));
    lines
}

fn phase_name(p: &FPhase) -> String {
    format!("{p:?}").split([' ', '{', '(']).next().unwrap_or("").to_string()
}

fn pair(a: [u8; 32], b: [u8; 32]) -> [u8; 32] {
    let (x, y) = if a <= b { (a, b) } else { (b, a) };
    etch_autobuyer::evmtx::keccak(&[x, y].concat())
}

fn step(n: &str, what: &str) {
    println!("\n== {n}. {what}");
}

fn main() {
    if let Err(e) = run() {
        println!("\nSIM: FAIL: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let var = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    let env = Env { rpc: var("SIM_RPC", "http://127.0.0.1:18743"), user: var("SIM_RPC_USER", "sim"), pass: var("SIM_RPC_PASS", "sim"),
        ord: var("SIM_ORD", "http://127.0.0.1:18780") };
    let inscription = std::env::var("SIM_INSCRIPTION").map_err(|_| "SIM_INSCRIPTION is not set")?;
    let market = var("SIM_MARKET", "ops/rehearsal/sim/market.mjs");
    let ops_dir = var("SIM_OPS_DIR", "ops");
    let price: u64 = var("SIM_PRICE_SATS", "200000").parse().map_err(|_| "SIM_PRICE_SATS")?;
    let cap_wei: u128 = 3 * E / 10;

    let float = TapKey::from_seed("rook-sim float (regtest only)");
    let custody = TapKey::from_seed("rook-sim custody (regtest only)");
    let float_addr = policy::taproot_address(&float.output, "bcrt");
    let custody_addr = policy::taproot_address(&custody.output, "bcrt");
    step("0", "the canister's two Bitcoin keys (STAND-IN: local BIP86 keys for the subnet's threshold Schnorr key)");
    println!("  float   {float_addr}  internal {}", hex::encode(float.internal));
    println!("  custody {custody_addr}  internal {}", hex::encode(custody.internal));

    // the collection: the sim's frog and a second member, the root the stubbed registry serves
    step("1", "the collection (members file → root, as ListingRegistry would hold it)");
    let other = format!("{}i0", "ab".repeat(32));
    let members = [inscription.clone(), other.clone()];
    let (l0, l1) = (frogbuy::leaf_of(&members[0]), frogbuy::leaf_of(&members[1]));
    let root = pair(l0, l1);
    let proof = vec![l1];
    println!("  members {:?}\n  root 0x{}", members, hex::encode(root));
    let (c, ins) = env.ord_get(&format!("/inscription/{inscription}"))?;
    if c != 200 {
        return Err(format!("ord does not know {inscription}"));
    }
    println!("  the frog (ord): {} at {} value {}", inscription, ins["satpoint"].as_str().unwrap_or(""), ins["value"]);

    let caps = {
        let k = custodycap::btc_key(&W);
        custodycap::CapPolicy { collections: vec![custodycap::CollectionCap { collection: k, max_items: 10, max_value_wei: 10 * E, max_item_wei: E }],
            global_max_items: 10, global_max_value_wei: 10 * E }
    };
    let mut io = LiveIo { env, float, custody, root, offers: HashMap::new(), saved: None, caps, book: custodycap::Book::default(),
        sent: vec![], signed: vec![] };
    let faucet_addr = io.env.rpc_at(Some("faucet"), "getnewaddress", json!(["mine", "bech32m"]))?.as_str().unwrap_or("").to_string();

    step("2", "BTC arrives at the float (STAND-IN for Relay's BTC leg: funded directly on regtest), 1 block");
    let mut funding = vec![];
    for amt in [600u64, 600, 500_000] {
        let t = io.env.rpc_at(Some("faucet"), "sendtoaddress", json!([float_addr, format!("{:.8}", amt as f64 / 1e8)]))?;
        println!("  faucet → float {amt} sats: {}", t.as_str().unwrap_or(""));
        funding.push(t);
    }
    io.env.mine(1, &faucet_addr)?;
    io.env.ord_synced()?;
    let fl = block_on(io.float_utxos())?;
    println!("  float UTXOs (scantxoutset): {}", fl.iter().map(|u| format!("{}:{}={}", hex::encode(u.outpoint.txid), u.outpoint.vout, u.value)).collect::<Vec<_>>().join(", "));

    step("3", "frog 0 is Wanted (STAND-IN for the floor trigger on 4663), cap 0.3 ETH; a tick with no offer");
    let mut st = FrogState { frogs: vec![FrogRec { id: 0, target: None, phase: FPhase::Wanted { cap_wei } }], next_frog: 1, ..Default::default() };
    tick(&mut io, &mut st, "no offer yet");

    step("4", "the listing and the runner's hand-in: the seller signs SINGLE|ANYONECANPAY, the marketplace builds the bid PSBT");
    let pads: Vec<&Utxo> = fl.iter().filter(|u| u.value == 600).collect();
    let pay = fl.iter().find(|u| u.value == 500_000).ok_or("no payment UTXO at the float")?;
    if pads.len() < 2 {
        return Err("the float has fewer than two 600-sat padding UTXOs".into());
    }
    let loc = block_on(io.locate(&inscription))?.ok_or("ord lost the frog")?;
    let (seller_out, _) = block_on(io.output(&loc.output))?;
    let u = |x: &Utxo| json!({"txid": hex::encode(x.outpoint.txid), "vout": x.outpoint.vout, "value": x.value});
    let arg = json!({
        "rpc": io.env.rpc, "user": io.env.user, "pass": io.env.pass, "seller_wallet": "ord",
        "seller": {"txid": hex::encode(loc.output.txid), "vout": loc.output.vout, "value": loc.value, "script": hex::encode(&seller_out.script)},
        "price": price, "fee": 1_500, "pads": [u(pads[0]), u(pads[1])], "pay": u(pay),
        "float_script": hex::encode(io.float.script()), "custody_script": hex::encode(io.custody.script()),
    });
    let out = Command::new("node").current_dir(&ops_dir).args([&market, &arg.to_string()]).output().map_err(|e| format!("node: {e}"))?;
    if !out.status.success() {
        return Err(format!("market.mjs failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    let m: Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("market.mjs: {e}: {}", String::from_utf8_lossy(&out.stdout)))?;
    let psbt = hex::decode(m["psbt_hex"].as_str().unwrap_or("")).map_err(|e| e.to_string())?;
    println!("  seller's listing witness (sighash 0x{}): {}…", &m["seller_witness"].as_str().unwrap_or("")[128..], &m["seller_witness"].as_str().unwrap_or("")[..24]);
    println!("  bid PSBT: {} bytes, to sign {}, seller paid to {}", psbt.len(), m["to_sign"], m["payout"].as_str().unwrap_or(""));
    if let Ok(dir) = std::env::var("SIM_OUT") {
        let _ = std::fs::write(format!("{dir}/bid.psbt.hex"), m["psbt_hex"].as_str().unwrap_or(""));
    }
    // 4b. the UniSat question (Part A): the SAME bid with the seller's witness stripped, as a marketplace that keeps the seller's
    //     signature server-side would hand it over. Handed in first; one tick shows what the canister does with it.
    step("4b", "the same bid WITHOUT the seller's signature (a marketplace that keeps it server-side): one tick");
    let parsed = etch_autobuyer::psbt::Psbt::parse(&psbt).map_err(|e| format!("{e:?}"))?;
    let stripped = frogbuy::psbt_bytes(&parsed, &vec![None; parsed.tx.inputs.len()]);
    io.offers.insert(0, Offer { inscription: inscription.clone(), psbt: stripped, to_sign: vec![0, 1, 3], proof: proof.clone() });
    let before = io.signed.len();
    tick(&mut io, &mut st, "seller-unsigned offer");
    println!("  float digests signed for it: {} (signatures discarded; nothing broadcast: {} sends so far)", io.signed.len() - before, io.sent.len());

    // what frog_offer stores: hex-decoded PSBT, to_sign, the membership proof
    io.offers.insert(0, Offer { inscription: inscription.clone(), psbt, to_sign: vec![0, 1, 3], proof: proof.clone() });
    println!("  frog_offer(0, {inscription}, <psbt>, [0,1,3], <proof>) held for the next tick");

    step("5", "the canister's tick: judge, sign the float's inputs, finalize, save, broadcast");
    tick(&mut io, &mut st, "offer in hand");
    let FPhase::Signed { buy } = st.frogs[0].phase.clone() else {
        return Err(format!("the offer was not bought: {:?}", st.frogs[0].phase));
    };
    let txid = hex::encode(buy.outpoint.txid);
    println!("  bought: spend {} sats (fee {}), cost {} wei, custody output {txid}:{}", buy.spend_sats, buy.fee_sats, buy.cost_wei, buy.outpoint.vout);
    println!("  float digests signed: {}", io.signed.len());
    let mp = io.env.rpc("getrawmempool", json!([]))?;
    println!("  mempool has the buy: {}", mp.as_array().is_some_and(|a| a.iter().any(|t| t.as_str() == Some(&txid))));

    step("6", "mine 1: the buy confirms");
    io.env.mine(1, &faucet_addr)?;
    io.env.ord_synced()?;
    tick(&mut io, &mut st, "1 deep");

    step("7", "mine 5 more: six deep, ord shows custody holding the frog");
    io.env.mine(5, &faucet_addr)?;
    io.env.ord_synced()?;
    tick(&mut io, &mut st, "6 deep");
    let held = matches!(st.frogs[0].phase, FPhase::Held { .. });

    step("8", "custody, read independently of the flow");
    let (_, now) = io.env.ord_get(&format!("/inscription/{inscription}"))?;
    let addr_now = now["address"].as_str().unwrap_or("").to_string();
    println!("  ord: {} now at {} ({}), owner {}", inscription, now["satpoint"].as_str().unwrap_or(""), now["value"], addr_now);
    let c = block_on(io.custody_utxos())?;
    println!("  custody UTXOs: {}", c.iter().map(|u| format!("{}:{}={}", hex::encode(u.outpoint.txid), u.outpoint.vout, u.value)).collect::<Vec<_>>().join(", "));
    let t = io.env.rpc("getrawtransaction", json!([txid, true]))?;
    println!("  buy tx {txid}: {} confirmations, {} inputs, {} outputs, vsize {}", t["confirmations"], t["vin"].as_array().map_or(0, |a| a.len()),
        t["vout"].as_array().map_or(0, |a| a.len()), t["vsize"]);
    if held && addr_now == custody_addr {
        println!("\nSIM: PASS — frog {inscription} bought by the canister's flow, six deep, held in custody {custody_addr}.");
        println!("SIM: stopped at Held. NOT simulated: the mint/settle on the EVM side (dropped: Rook moves to Solana), Relay, the IC itself.");
        Ok(())
    } else {
        Err(format!("the frog is not held: phase {:?}, ord owner {addr_now}", phase_name(&st.frogs[0].phase)))
    }
}
