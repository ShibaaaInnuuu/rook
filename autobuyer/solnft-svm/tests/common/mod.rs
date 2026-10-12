//! The LiteSVM harness: the Metaplex programs (byte-identical to mainnet, `solana/tests/fixtures`), the marketplace programs dumped
//! from mainnet (`fixtures/programs`), the mainnet accounts of one recorded scenario
//! (`autobuyer/canister-rs/tests/fixtures/solnft/<scenario>.json`), the clock at its slot, and a stand-in canister wallet. The
//! canister's own code builds, judges, compiles (`soltx`) and assembles every transaction; LiteSVM only signs nothing and runs it.
#![allow(dead_code)]

use std::path::PathBuf;

use base64::Engine;
use etch_autobuyer::solnft::{self, Accounts};
use etch_autobuyer::solrpc::Account as CAccount;
use etch_autobuyer::soltx::{self, Instruction as CIx, Pubkey as CKey};
use litesvm::LiteSVM;
use solana_account::Account;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::versioned::VersionedTransaction;

pub const TENSOR: &str = "TCMPhJdwDryooaGtiocG1u3xcYbRpiJzb283XfCZsDp";
pub const MMM: &str = "mmm3XBJg5gk8XJxEKBvdgptZz6SgK4tXvn36sodowMc";
pub const M2: &str = "M2mx93ekt1fmXSVkTrUL9xVFHkmME8HTUi5Cyc5aF7K";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn pk(k: &CKey) -> Pubkey {
    Pubkey::new_from_array(*k)
}

pub fn ck(k: &Pubkey) -> CKey {
    k.to_bytes()
}

pub struct Fixture {
    pub slot: u64,
    pub unix_time: i64,
    pub facts: serde_json::Value,
    pub accounts: Vec<(CKey, CAccount)>,
    pub absent: Vec<CKey>,
}

pub fn fixture(name: &str) -> Fixture {
    let p = root().join("../canister-rs/tests/fixtures/solnft").join(format!("{name}.json"));
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD;
    let accounts = j["accounts"].as_array().unwrap().iter().map(|a| {
        (soltx::pubkey(a["pubkey"].as_str().unwrap()).unwrap(), CAccount {
            lamports: a["lamports"].as_u64().unwrap(),
            owner: soltx::pubkey(a["owner"].as_str().unwrap()).unwrap(),
            executable: a["executable"].as_bool().unwrap(),
            data: b64.decode(a["data"].as_str().unwrap()).unwrap(),
        })
    }).collect();
    let absent = j["absent"].as_array().unwrap().iter().map(|k| soltx::pubkey(k.as_str().unwrap()).unwrap()).collect();
    Fixture { slot: j["slot"].as_u64().unwrap(), unix_time: j["unix_time"].as_i64().unwrap(), facts: j["facts"].clone(), accounts, absent }
}

impl Fixture {
    /// The canister's view of the scenario (as `solnft::read` would return it at that slot).
    pub fn view(&self) -> Accounts {
        let mut a: Accounts = self.accounts.iter().map(|(k, v)| (*k, Some(v.clone()))).collect();
        for k in &self.absent {
            a.insert(*k, None);
        }
        a
    }
    pub fn fact(&self, k: &str) -> CKey {
        soltx::pubkey(self.facts[k].as_str().unwrap_or_else(|| panic!("fact {k}"))).unwrap()
    }
}

pub struct Env {
    pub svm: LiteSVM,
    /// The stand-in canister wallet (in production: the canister's threshold-Ed25519 key).
    pub canister: Keypair,
}

pub fn env(f: &Fixture) -> Env {
    let mut svm = LiteSVM::new();
    let metaplex = root().join("../../solana/tests/fixtures");
    for (id, file) in [
        (solnft::TOKEN_METADATA, metaplex.join("mpl_token_metadata.so")),
        (solnft::MPL_CORE, metaplex.join("mpl_core.so")),
        (solnft::TOKEN_AUTH_RULES, metaplex.join("mpl_token_auth_rules.so")),
        (TENSOR, root().join("fixtures/programs/tensor_marketplace.so")),
        (MMM, root().join("fixtures/programs/mmm.so")),
        (M2, root().join("fixtures/programs/m2.so")),
    ] {
        svm.add_program_from_file(pk(&soltx::key(id)), file).unwrap();
    }
    for (k, a) in &f.accounts {
        svm.set_account(pk(k), Account { lamports: a.lamports, data: a.data.clone(), owner: pk(&a.owner), executable: false, rent_epoch: u64::MAX })
            .unwrap();
    }
    let mut clock: solana_clock::Clock = svm.get_sysvar();
    clock.unix_timestamp = f.unix_time;
    clock.slot = f.slot;
    svm.set_sysvar(&clock);
    let canister = Keypair::new();
    svm.airdrop(&canister.pubkey(), 100_000_000_000).unwrap();
    Env { svm, canister }
}

impl Env {
    pub fn me(&self) -> CKey {
        ck(&self.canister.pubkey())
    }
    /// Compile with the canister's own compiler, sign (the stand-in key; the canister signs the same bytes), assemble, run.
    pub fn run(&mut self, ixs: &[CIx], extra: &[&Keypair]) -> Result<litesvm::types::TransactionMetadata, String> {
        let bh = self.svm.latest_blockhash().to_bytes();
        let parts = soltx::compile_parts(ixs, self.me(), bh)?;
        let msg = parts.message();
        let mut sigs = Vec::new();
        for s in &msg.signers {
            let kp = std::iter::once(&self.canister).chain(extra.iter().copied()).find(|k| ck(&k.pubkey()) == *s)
                .ok_or_else(|| format!("no key for signer {}", soltx::b58_encode(s)))?;
            sigs.push(<[u8; 64]>::try_from(kp.sign_message(&msg.bytes).as_ref()).unwrap());
        }
        let tx = soltx::assemble(&msg, &sigs)?;
        let vt: VersionedTransaction = bincode::deserialize(&tx).map_err(|e| e.to_string())?;
        let r = self.svm.send_transaction(vt).map_err(|e| format!("{:?}\n{}", e.err, e.meta.logs.join("\n")))?;
        self.svm.expire_blockhash();
        Ok(r)
    }
    pub fn lamports(&self, k: &CKey) -> u64 {
        self.svm.get_account(&pk(k)).map(|a| a.lamports).unwrap_or(0)
    }
    /// The canister's view now, of `keys`.
    pub fn view(&self, keys: &[CKey]) -> Accounts {
        keys.iter().map(|k| (*k, self.svm.get_account(&pk(k)).filter(|a| a.lamports > 0 || !a.data.is_empty())
            .map(|a| CAccount { lamports: a.lamports, owner: ck(&a.owner), executable: a.executable, data: a.data }))).collect()
    }
}
