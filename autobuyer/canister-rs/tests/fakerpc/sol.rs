//! A fake Solana provider pair (docs/v9/solana-migration.md §4, "tests/fakerpc gains a fake Solana provider pair"): one chain
//! ([`SolChain`]: finalized accounts, blocks, signature statuses, durable nonces, the transactions sent), served by two providers
//! that answer JSON-RPC the way real ones did on devnet (spike Q3: `context.slot` and `apiVersion` differ between them, statuses
//! carry `confirmations`, a skipped slot is `-32007`, a resent landed durable-nonce transaction is `BlockhashNotFound`) and run the
//! canister's own transform (`solrpc::normalise`) on each answer, as the IC does. A provider can lie about an account, lag a
//! status, or fail; a hook can change the chain between two reads (the ordering race of Q3 §4.3).
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use base64::Engine;
use etch_autobuyer::solkey::{KeyPath, SolSigner};
use etch_autobuyer::solrpc::{context, normalise, Account, Kind, Level, Req, SolRpc};
use etch_autobuyer::soltx::{self, Hash, Pubkey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Runs before a provider answers: may change the chain between two reads.
pub type Hook = Rc<RefCell<Option<Box<dyn FnMut(&Req, &mut SolChain)>>>>;

/// What a landed transaction does to the chain's accounts (phase B1: a fake rook_core, `fakerpc::rook`): its instructions (the nonce
/// advance left out), its fee payer, the accounts to change, the chain's unix time. An `Err` is the transaction's on-chain error: it
/// lands failed and changes nothing (its nonce still moves, its fee is still paid, as on Solana).
pub type Exec = Box<dyn FnMut(&[soltx::Instruction], &Pubkey, &mut BTreeMap<Pubkey, Account>, u64) -> Result<(), Value>>;

/// What a transaction WOULD do (phase B2: `simulateTransaction`, before the canister signs): the same execution on copies, nothing kept.
pub type Sim = Box<dyn Fn(&[soltx::Instruction], &Pubkey, &BTreeMap<Pubkey, Account>, u64) -> Result<(), Value>>;

/// The fee a landed transaction's payer pays.
pub const FEE_LAMPORTS: u64 = 5_000;

/// One landed signature.
#[derive(Clone, Debug)]
pub struct Landed {
    pub slot: u64,
    pub err: Option<Value>,
    pub level: Level,
}

/// The chain both fake providers serve. Every state here is `finalized` unless a landing says otherwise.
pub struct SolChain {
    pub slot: u64,
    pub height: u64,
    pub accounts: BTreeMap<Pubkey, Account>,
    /// slot → (blockhash, block height); a slot missing here is skipped.
    pub blocks: BTreeMap<u64, (Hash, u64)>,
    pub landed: BTreeMap<String, Landed>,
    /// Every transaction any provider was sent, in order (duplicates included).
    pub sent: Vec<Vec<u8>>,
    /// The error the next landing carries (a transaction that lands and fails).
    pub fail_next: Option<Value>,
    nonce_moves: u64,
    /// The chain's unix time (`getBlockTime` of any slot).
    pub unix: u64,
    /// What landing a transaction does (none: nothing but its nonce, as before).
    pub exec: Option<Exec>,
    /// Finalized transactions others made (a Relay refund memoed with its order): `(signature, slot, getTransaction's json)`.
    pub txs: Vec<(String, u64, Value)>,
    /// Every provider's preflight refuses a NEW transaction with this simulation error (not forwarded: it never reaches `sent`).
    pub preflight_err: Option<Value>,
    /// What a simulation runs (none: a simulation only checks the lifetime and the fee payer).
    pub sim: Option<Sim>,
    /// Every transaction simulated, in order (phase B2: simulated before it is signed).
    pub simulated: Vec<Vec<u8>>,
    /// Every provider's simulation fails with this error (the next ones, until cleared).
    pub sim_err: Option<Value>,
    /// The cluster's genesis hash (devnet's by default).
    pub genesis: String,
}

pub fn hash_of(tag: &str, n: u64) -> Hash {
    Sha256::digest(format!("{tag}{n}").as_bytes()).into()
}

impl SolChain {
    pub fn new() -> Rc<RefCell<SolChain>> {
        let mut c = SolChain { slot: 1_000, height: 900, accounts: BTreeMap::new(), blocks: BTreeMap::new(), landed: BTreeMap::new(),
            sent: vec![], fail_next: None, nonce_moves: 0, unix: 1_791_565_840, exec: None, txs: vec![], preflight_err: None, sim: None, simulated: vec![], sim_err: None,
            genesis: "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG".into() };
        c.blocks.insert(1_000, (hash_of("block", 1_000), 900));
        Rc::new(RefCell::new(c))
    }

    /// `n` more slots, each a block (one more height) unless listed in `skipped`.
    pub fn advance(&mut self, n: u64, skipped: &[u64]) {
        for _ in 0..n {
            self.slot += 1;
            if !skipped.contains(&self.slot) {
                self.height += 1;
                self.blocks.insert(self.slot, (hash_of("block", self.slot), self.height));
            }
        }
    }

    pub fn set_account(&mut self, key: Pubkey, owner: Pubkey, lamports: u64, data: Vec<u8>) {
        self.accounts.insert(key, Account { lamports, owner, executable: false, data });
    }

    /// An initialized durable-nonce account.
    pub fn set_nonce(&mut self, key: Pubkey, authority: Pubkey) -> Hash {
        let n = hash_of("nonce", self.nonce_moves);
        self.nonce_moves += 1;
        let st = soltx::NonceState { authority, durable_nonce: n, lamports_per_signature: 5_000 };
        self.set_account(key, soltx::SYSTEM_PROGRAM, 1_447_680, soltx::encode_nonce(&st));
        n
    }

    pub fn nonce_of(&self, key: &Pubkey) -> soltx::NonceState {
        soltx::decode_nonce(&self.accounts[key].data).unwrap()
    }

    /// Someone (not the canister's transaction) advances the nonce: a bug or a compromised key (Q3 §4.4).
    pub fn move_nonce(&mut self, key: &Pubkey) -> Hash {
        let auth = self.nonce_of(key).authority;
        self.set_nonce(*key, auth)
    }

    /// Land what can land, in the order sent, at `level`: a durable-nonce transaction whose blockhash is its nonce account's
    /// current value (and advance that nonce), a blockhash transaction whose blockhash is ≤ 150 blocks old. Each signature once.
    pub fn process(&mut self, level: Level) -> Vec<String> {
        let mut out = vec![];
        for tx in self.sent.clone() {
            let (sigs, msg) = soltx::split_tx(&tx).unwrap();
            let sig = soltx::b58_encode(&sigs[0]);
            if self.landed.contains_key(&sig) {
                continue;
            }
            let p = soltx::Parts::parse(msg).unwrap();
            let first = &p.instructions[0];
            let nonced = p.keys[first.program as usize] == soltx::SYSTEM_PROGRAM && first.data == [4, 0, 0, 0];
            let ok = if nonced {
                let nonce_key = p.keys[first.accounts[0] as usize];
                match self.accounts.get(&nonce_key).and_then(|a| soltx::decode_nonce(&a.data).ok()) {
                    Some(n) if n.durable_nonce == p.blockhash => {
                        self.move_nonce(&nonce_key);
                        true
                    }
                    _ => false,
                }
            } else {
                self.blocks.values().any(|(h, height)| *h == p.blockhash && self.height <= height + 150)
            };
            if ok {
                let mut err = self.fail_next.take();
                if err.is_none() && self.exec.is_some() {
                    let payer = p.signers()[0];
                    if let Some(a) = self.accounts.get_mut(&payer) {
                        a.lamports = a.lamports.saturating_sub(FEE_LAMPORTS);
                    }
                    let ixs = self.instructions_of(&p);
                    let rest = if nonced { &ixs[1..] } else { &ixs[..] };
                    let mut next = self.accounts.clone();
                    let unix = self.unix;
                    match (self.exec.as_mut().unwrap())(rest, &payer, &mut next, unix) {
                        Ok(()) => self.accounts = next,
                        Err(e) => err = Some(e),
                    }
                }
                self.landed.insert(sig.clone(), Landed { slot: self.slot, err, level });
                out.push(sig);
            }
        }
        out
    }

    /// `simulateTransaction` of an unsigned transaction against the current state: the lifetime (its nonce or blockhash), the fee
    /// payer's balance, then the instructions on copies of the accounts. `None`: it would succeed.
    pub fn simulate(&self, tx: &[u8]) -> Option<Value> {
        let (_, msg) = soltx::split_tx(tx).unwrap();
        let p = soltx::Parts::parse(msg).unwrap();
        let first = &p.instructions[0];
        let nonced = p.keys[first.program as usize] == soltx::SYSTEM_PROGRAM && first.data == [4, 0, 0, 0];
        let live = if nonced {
            let nonce_key = p.keys[first.accounts[0] as usize];
            self.accounts.get(&nonce_key).and_then(|a| soltx::decode_nonce(&a.data).ok()).is_some_and(|n| n.durable_nonce == p.blockhash)
        } else {
            self.blocks.values().any(|(h, height)| *h == p.blockhash && self.height <= height + 150)
        };
        if !live {
            return Some(json!("BlockhashNotFound"));
        }
        let payer = p.signers()[0];
        match self.accounts.get(&payer) {
            None => return Some(json!("AccountNotFound")),
            Some(a) if a.lamports < FEE_LAMPORTS => return Some(json!("InsufficientFundsForFee")),
            _ => {}
        }
        let Some(sim) = self.sim.as_ref() else { return None };
        let ixs = self.instructions_of(&p);
        let rest = if nonced { &ixs[1..] } else { &ixs[..] };
        let mut acc = self.accounts.clone();
        if let Some(a) = acc.get_mut(&payer) {
            a.lamports -= FEE_LAMPORTS;
        }
        sim(rest, &payer, &acc, self.unix).err()
    }

    /// A message's instructions, a v0 message's keys loaded from the lookup tables on this chain (as a node loads them).
    pub fn instructions_of(&self, p: &soltx::Parts) -> Vec<soltx::Instruction> {
        let tables: Vec<soltx::LookupTable> = p.tables.iter().filter_map(|u| self.accounts.get(&u.table)
            .and_then(|a| soltx::decode_lookup_table(&a.data).ok()).map(|t| soltx::LookupTable { key: u.table, addresses: t.addresses })).collect();
        let (w, r) = soltx::loaded_keys(p, &tables).unwrap();
        p.decompile(&w, &r).unwrap()
    }

    pub fn finalize_all(&mut self) {
        for l in self.landed.values_mut() {
            l.level = Level::Finalized;
        }
    }

    /// Distinct transactions sent.
    pub fn distinct_sent(&self) -> usize {
        let mut v = self.sent.clone();
        v.sort();
        v.dedup();
        v.len()
    }
}

fn account_json(a: &Account) -> Value {
    json!({ "data": [base64::engine::general_purpose::STANDARD.encode(&a.data), "base64"], "executable": a.executable,
        "lamports": a.lamports, "owner": soltx::b58_encode(&a.owner), "rentEpoch": u64::MAX, "space": a.data.len() })
}

/// One fake provider over the shared chain.
pub struct FakeSol {
    pub name: &'static str,
    pub chain: Rc<RefCell<SolChain>>,
    pub api: &'static str,
    /// How far this provider's `context.slot` runs ahead (the field the transform drops).
    pub ctx_ahead: u64,
    /// Accounts this provider answers differently (a liar, or a node behind).
    pub lie: BTreeMap<Pubkey, Option<Account>>,
    /// Report every finalized signature as `confirmed` (a provider a slot behind).
    pub status_behind: bool,
    /// The outcall fails.
    pub down: bool,
    /// Every request either provider answered, in order: `method` or `method:<first param>`.
    pub asked: Rc<RefCell<Vec<String>>>,
    pub hook: Hook,
}

/// The measured pair (devnet, 2026-10-09): `api.devnet.solana.com` and OnFinality, on different versions.
pub fn pair(chain: &Rc<RefCell<SolChain>>) -> Vec<FakeSol> {
    let asked = Rc::new(RefCell::new(vec![]));
    let hook: Hook = Rc::new(RefCell::new(None));
    let p = |name, api, ctx_ahead| FakeSol { name, chain: chain.clone(), api, ctx_ahead, lie: BTreeMap::new(), status_behind: false,
        down: false, asked: asked.clone(), hook: hook.clone() };
    vec![p("api.devnet.solana.com", "4.4.0-beta.0", 0), p("solana-devnet.api.onfinality.io", "3.0.1", 36)]
}

impl FakeSol {
    fn ctx(&self, c: &SolChain) -> Value {
        json!({ "apiVersion": self.api, "slot": c.slot + self.ctx_ahead })
    }

    fn raw(&self, r: &Req) -> Value {
        let c = self.chain.borrow();
        let p0 = &r.params[0];
        let ok = |result: Value| json!({ "jsonrpc": "2.0", "result": result, "id": 1 });
        let err = |code: i64, message: &str, data: Value| json!({ "jsonrpc": "2.0", "error": { "code": code, "message": message, "data": data }, "id": 1 });
        match r.method {
            "getSlot" => ok(json!(c.slot)),
            "getBlockHeight" => ok(json!(c.height)),
            "getBlock" => match c.blocks.get(&p0.as_u64().unwrap()) {
                Some((h, height)) => ok(json!({ "blockHeight": height, "blockTime": 1_791_565_840u64, "blockhash": soltx::b58_encode(h),
                    "parentSlot": p0.as_u64().unwrap() - 1, "previousBlockhash": soltx::b58_encode(&hash_of("prev", 0)) })),
                None => err(-32007, &format!("Slot {} was skipped, or missing due to ledger jump to recent snapshot", p0), Value::Null),
            },
            "getAccountInfo" => {
                let k = soltx::pubkey(p0.as_str().unwrap()).unwrap();
                let mut a = self.lie.get(&k).cloned().unwrap_or_else(|| c.accounts.get(&k).cloned());
                if let (Some(x), Some(sl)) = (a.as_mut(), r.params[1].get("dataSlice")) {
                    let (o, l) = (sl["offset"].as_u64().unwrap() as usize, sl["length"].as_u64().unwrap() as usize);
                    x.data = x.data.get(o..(o + l).min(x.data.len())).unwrap_or_default().to_vec();
                }
                ok(json!({ "context": self.ctx(&c), "value": a.as_ref().map(account_json) }))
            }
            "getProgramAccounts" => {
                let program = soltx::pubkey(p0.as_str().unwrap()).unwrap();
                let filters = r.params[1]["filters"].as_array().cloned().unwrap_or_default();
                let v: Vec<Value> = c.accounts.iter().filter(|(_, a)| a.owner == program).filter(|(_, a)| filters.iter().all(|f| {
                    if let Some(n) = f.get("dataSize") {
                        return a.data.len() as u64 == n.as_u64().unwrap();
                    }
                    let m = &f["memcmp"];
                    let (o, b) = (m["offset"].as_u64().unwrap() as usize, soltx::b58_decode(m["bytes"].as_str().unwrap()).unwrap());
                    a.data.get(o..o + b.len()) == Some(&b[..])
                })).map(|(k, a)| {
                    let mut a = a.clone();
                    if let Some(sl) = r.params[1].get("dataSlice") {
                        let (o, l) = (sl["offset"].as_u64().unwrap() as usize, sl["length"].as_u64().unwrap() as usize);
                        a.data = a.data.get(o..(o + l).min(a.data.len())).unwrap_or_default().to_vec();
                    }
                    json!({ "pubkey": soltx::b58_encode(k), "account": account_json(&a) })
                }).rev().collect();
                ok(json!(v))
            }
            "getBlockTime" => ok(json!(c.unix)),
            "getGenesisHash" => ok(json!(c.genesis)),
            "simulateTransaction" => {
                let tx = base64::engine::general_purpose::STANDARD.decode(p0.as_str().unwrap()).unwrap();
                assert_eq!(r.params[1]["sigVerify"], false, "an unsigned transaction is simulated without signature checks");
                let e = c.sim_err.clone().or_else(|| c.preflight_err.clone()).or_else(|| c.simulate(&tx));
                drop(c);
                self.chain.borrow_mut().simulated.push(tx);
                let c = self.chain.borrow();
                ok(json!({ "context": self.ctx(&c), "value": { "err": e, "logs": ["Program log: simulated"], "accounts": null,
                    "unitsConsumed": 1234, "returnData": null } }))
            }
            "getSignaturesForAddress" => {
                let addr = p0.as_str().unwrap();
                let until = r.params[1]["until"].as_str().map(String::from);
                let limit = r.params[1]["limit"].as_u64().unwrap_or(1_000) as usize;
                let mut v = vec![];
                for (sig, slot, tx) in c.txs.iter().rev() {
                    if Some(sig) == until.as_ref() {
                        break;
                    }
                    if tx["transaction"]["message"]["accountKeys"].as_array().unwrap().iter().any(|k| k == addr) {
                        v.push(json!({ "signature": sig, "slot": slot, "err": tx["meta"]["err"], "memo": null, "blockTime": c.unix,
                            "confirmationStatus": "finalized" }));
                    }
                }
                v.truncate(limit);
                ok(json!(v))
            }
            "getTransaction" => {
                let sig = p0.as_str().unwrap();
                ok(c.txs.iter().find(|(s, ..)| s == sig).map(|(_, slot, t)| {
                    let mut t = t.clone();
                    t["slot"] = json!(slot);
                    t["blockTime"] = json!(c.unix);
                    t
                }).unwrap_or(Value::Null))
            }
            "getMultipleAccounts" => {
                let v: Vec<Value> = p0.as_array().unwrap().iter().map(|s| {
                    let k = soltx::pubkey(s.as_str().unwrap()).unwrap();
                    let mut a = self.lie.get(&k).cloned().unwrap_or_else(|| c.accounts.get(&k).cloned());
                    // D-i: the registry keeper reads a field of many accounts (`dataSlice`)
                    if let (Some(x), Some(sl)) = (a.as_mut(), r.params[1].get("dataSlice")) {
                        let (o, l) = (sl["offset"].as_u64().unwrap() as usize, sl["length"].as_u64().unwrap() as usize);
                        x.data = x.data.get(o..(o + l).min(x.data.len())).unwrap_or_default().to_vec();
                    }
                    a.as_ref().map(account_json).unwrap_or(Value::Null)
                }).collect();
                ok(json!({ "context": self.ctx(&c), "value": v }))
            }
            "getTokenAccountsByOwner" => {
                // D-i: a token program's accounts whose owner field (bytes 32..64) is the address asked for, sliced
                let owner = soltx::pubkey(p0.as_str().unwrap()).unwrap();
                let program = soltx::pubkey(r.params[1]["programId"].as_str().unwrap()).unwrap();
                let v: Vec<Value> = c.accounts.iter().filter(|(_, a)| a.owner == program && a.data.get(32..64) == Some(&owner[..])).map(|(k, a)| {
                    let mut a = a.clone();
                    if let Some(sl) = r.params[2].get("dataSlice") {
                        let (o, l) = (sl["offset"].as_u64().unwrap() as usize, sl["length"].as_u64().unwrap() as usize);
                        a.data = a.data.get(o..(o + l).min(a.data.len())).unwrap_or_default().to_vec();
                    }
                    json!({ "pubkey": soltx::b58_encode(k), "account": account_json(&a) })
                }).collect();
                ok(json!({ "context": self.ctx(&c), "value": v }))
            }
            "getBalance" => {
                let k = soltx::pubkey(p0.as_str().unwrap()).unwrap();
                ok(json!({ "context": self.ctx(&c), "value": c.accounts.get(&k).map(|a| a.lamports).unwrap_or(0) }))
            }
            "getMinimumBalanceForRentExemption" => ok(json!(6_960 * (128 + p0.as_u64().unwrap()))),
            "getSignatureStatuses" => {
                let v: Vec<Value> = p0.as_array().unwrap().iter().map(|s| match c.landed.get(s.as_str().unwrap()) {
                    None => Value::Null,
                    Some(l) => {
                        let level = if self.status_behind && l.level == Level::Finalized { Level::Confirmed } else { l.level };
                        let name = match level { Level::Processed => "processed", Level::Confirmed => "confirmed", Level::Finalized => "finalized" };
                        let status = match &l.err { None => json!({ "Ok": null }), Some(e) => json!({ "Err": e }) };
                        json!({ "slot": l.slot, "confirmations": if level == Level::Finalized { Value::Null } else { json!(c.slot - l.slot) },
                            "err": l.err.clone().unwrap_or(Value::Null), "status": status, "confirmationStatus": name })
                    }
                }).collect();
                ok(json!({ "context": self.ctx(&c), "value": v }))
            }
            "sendTransaction" => {
                let tx = base64::engine::general_purpose::STANDARD.decode(p0.as_str().unwrap()).unwrap();
                let preflight = r.params[1]["skipPreflight"] == false;
                if let (Some(e), true) = (c.preflight_err.clone(), preflight) {
                    // the simulation fails: the provider forwards nothing
                    let msg = if e.to_string().contains("InstructionError") { "Transaction simulation failed: Error processing Instruction 1: custom program error" }
                        else { "Transaction simulation failed: Attempt to debit an account but found no record of a prior credit." };
                    return err(-32002, msg, json!({ "err": e, "logs": [], "unitsConsumed": 0 }));
                }
                drop(c);
                self.chain.borrow_mut().sent.push(tx.clone());
                let c = self.chain.borrow();
                let (sigs, _) = soltx::split_tx(&tx).unwrap();
                let sig = soltx::b58_encode(&sigs[0]);
                if c.landed.contains_key(&sig) {
                    // a landed durable-nonce transaction resent: its nonce moved, so the age check fails first (Q3 §4.2)
                    err(-32002, "Transaction simulation failed: Blockhash not found", json!({ "err": "BlockhashNotFound", "logs": [], "unitsConsumed": 0 }))
                } else {
                    ok(json!(sig))
                }
            }
            m => panic!("the fake does not serve {m}"),
        }
    }
}

impl SolRpc for FakeSol {
    fn name(&self) -> String {
        self.name.into()
    }
    async fn ask(&mut self, r: &Req) -> Result<Value, String> {
        let first = r.params.get(0).map(|p| match p {
            Value::String(s) if s.len() <= 44 => format!(":{s}"),
            Value::Number(n) => format!(":{n}"),
            _ => String::new(),
        }).unwrap_or_default();
        self.asked.borrow_mut().push(format!("{}{first}", r.method));
        if let Some(h) = self.hook.borrow_mut().as_mut() {
            h(r, &mut self.chain.borrow_mut());
        }
        if self.down {
            return Err("SysTransient: Connecting to the provider failed".into());
        }
        let body = self.raw(r).to_string();
        // the IC refuses an answer over the request's `max_response_bytes` (the body alone here; a real one counts headers too)
        if body.len() as u64 > r.max_bytes {
            return Err(format!("SysFatal: Http body exceeds size limit of {} bytes", r.max_bytes));
        }
        Ok(normalise(200, body.as_bytes(), &context(r.kind, r.bucket)))
    }
}

/// A signer that is not a threshold key: deterministic 64 bytes per message, every message it signed recorded. It also holds the
/// dedicated Relay deposit key (audit A5-03), derived from `me` ([`relay_of`]).
pub struct FakeSigner {
    pub me: Pubkey,
    pub relay: Pubkey,
    pub signed: Rc<RefCell<Vec<Vec<u8>>>>,
}

/// The fake's Relay deposit key for `me`.
pub fn relay_of(me: &Pubkey) -> Pubkey {
    Sha256::digest([b"sol-relay".as_slice(), me].concat()).into()
}

impl FakeSigner {
    pub fn new(me: Pubkey) -> FakeSigner {
        FakeSigner { me, relay: relay_of(&me), signed: Rc::new(RefCell::new(vec![])) }
    }
}

impl SolSigner for FakeSigner {
    fn me(&self) -> Pubkey {
        self.me
    }
    async fn sign_bytes(&mut self, message: &[u8]) -> Result<[u8; 64], String> {
        self.signed.borrow_mut().push(message.to_vec());
        let a: [u8; 32] = Sha256::digest(message).into();
        let b: [u8; 32] = Sha256::digest(a).into();
        Ok([a, b].concat().try_into().unwrap())
    }
    fn relay(&self) -> Option<Pubkey> {
        Some(self.relay)
    }
    async fn sign_relay_bytes(&mut self, message: &[u8]) -> Result<[u8; 64], String> {
        self.signed.borrow_mut().push(message.to_vec());
        let a: [u8; 32] = Sha256::digest([b"relay".as_slice(), message].concat()).into();
        let b: [u8; 32] = Sha256::digest(a).into();
        Ok([a, b].concat().try_into().unwrap())
    }
    /// W8: the canister's derived keys (`K_coll[i]`, …), fake: a hash of the path (the fake chain checks signer flags, not signatures).
    async fn key(&mut self, path: &KeyPath) -> Result<Pubkey, String> {
        Ok(match path {
            KeyPath::Main => self.me,
            KeyPath::Relay => self.relay,
            p => derived_key(p),
        })
    }
    async fn sign_paths(&mut self, paths: &[KeyPath], message: &[u8]) -> Vec<Result<[u8; 64], String>> {
        let mut out = vec![];
        for p in paths {
            out.push(match p {
                KeyPath::Main => self.sign_bytes(message).await,
                KeyPath::Relay => self.sign_relay_bytes(message).await,
                p => {
                    self.signed.borrow_mut().push(message.to_vec());
                    let a: [u8; 32] = Sha256::digest([p.derivation().concat(), message.to_vec()].concat()).into();
                    let b: [u8; 32] = Sha256::digest(a).into();
                    Ok([a, b].concat().try_into().unwrap())
                }
            });
        }
        out
    }
}

/// The fake's address of a derived key path.
pub fn derived_key(p: &KeyPath) -> Pubkey {
    Sha256::digest([b"sol-derived".to_vec(), p.derivation().concat()].concat()).into()
}

pub fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::SlotBucket => "slot",
        Kind::Account => "account",
        Kind::SigStatus => "status",
        Kind::Send => "send",
        Kind::Block => "block",
        Kind::Number => "number",
        Kind::HeightBucket => "height",
        Kind::Accounts => "accounts",
        Kind::ProgramAccounts => "program_accounts",
        Kind::Signatures => "signatures",
        Kind::Transaction => "transaction",
        Kind::Simulate => "simulate",
        Kind::Text => "text",
        Kind::ProgramSlices => "program_slices",
    }
}
