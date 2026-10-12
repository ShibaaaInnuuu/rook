//! What the fake Solana chain EXECUTES when it lands a transaction on the books home (docs/v9/canister-only-solana.md §8; `SolChain::exec`):
//! the System Program's transfer, create-with-seed, allocate-with-seed and nonce initialization (the money line's nonce account, the
//! advance `M → K_main` by the collection's key, the way back), and Relay's `deposit_native` (lamports into Relay's vault, the deposit
//! logged with its order id). Every rook_core instruction the earlier fake ran is a book update now (decision D-l: no program).
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use etch_autobuyer::relaysol;
use etch_autobuyer::solrpc::Account;
use etch_autobuyer::soltx::{self, Instruction, Pubkey};
use serde_json::{json, Value};

use super::sol::SolChain;

pub const ONE: u64 = 1_000_000_000;
/// Every data account's lamports (rent is not what these tests are about).
pub const RENT: u64 = 2_000_000;

#[derive(Clone, Debug, Default)]
pub struct SysWorld {
    /// Relay deposits that landed: `(sender, lamports, order id)`.
    pub deposits: Vec<(Pubkey, u64, [u8; 32])>,
    /// Every instruction executed, by name, in order (`system:<tag>`, `deposit_native`).
    pub log: Vec<String>,
}

fn rd_u64(d: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(d[at..at + 8].try_into().unwrap())
}

fn err(e: &str) -> Value {
    json!({ "InstructionError": [0, { "Custom": e }] })
}

fn pay(accounts: &mut BTreeMap<Pubkey, Account>, from: &Pubkey, to: &Pubkey, lamports: u64) -> Result<(), Value> {
    let f = accounts.get_mut(from).ok_or_else(|| err("no payer account"))?;
    if f.lamports < lamports {
        return Err(json!({ "InstructionError": [0, "InsufficientFunds"] }));
    }
    f.lamports -= lamports;
    let t = accounts.entry(*to).or_insert(Account { lamports: 0, owner: soltx::SYSTEM_PROGRAM, executable: false, data: vec![] });
    t.lamports += lamports;
    Ok(())
}

impl SysWorld {
    /// Execute one landed transaction's instructions (`SolChain::exec`).
    pub fn exec(&mut self, ixs: &[Instruction], a: &mut BTreeMap<Pubkey, Account>, now: u64) -> Result<(), Value> {
        for ix in ixs {
            if ix.program == soltx::key(relaysol::DEPOSITORY_PROGRAM) {
                let (amount, id) = relaysol::decode_deposit_native(&ix.data).ok_or_else(|| err("not deposit_native"))?;
                let sender = ix.accounts[1].key;
                pay(a, &sender, &soltx::key(relaysol::VAULT), amount)?;
                self.deposits.push((sender, amount, id));
                self.log.push("deposit_native".into());
            } else if ix.program == soltx::SYSTEM_PROGRAM {
                let tag = u32::from_le_bytes(ix.data[..4].try_into().unwrap());
                match tag {
                    2 => pay(a, &ix.accounts[0].key, &ix.accounts[1].key, rd_u64(&ix.data, 4))?,
                    3 => {
                        // create_with_seed: the payer funds the new account (owner at the data's end)
                        let n = ix.data.len();
                        let owner: Pubkey = ix.data[n - 32..].try_into().unwrap();
                        let space = rd_u64(&ix.data, n - 40) as usize;
                        let lamports = rd_u64(&ix.data, n - 48);
                        let k = ix.accounts[1].key;
                        if a.contains_key(&k) {
                            return Err(err("AccountAlreadyInUse"));
                        }
                        pay(a, &ix.accounts[0].key, &k, lamports)?;
                        let acc = a.get_mut(&k).unwrap();
                        acc.owner = owner;
                        acc.data = vec![0; space];
                    }
                    9 => {
                        // allocate_with_seed (audit A4-02): a System-owned account with lamports and no data gets its space
                        let n = ix.data.len();
                        let owner: Pubkey = ix.data[n - 32..].try_into().unwrap();
                        let space = rd_u64(&ix.data, n - 40) as usize;
                        let acc = a.get_mut(&ix.accounts[0].key).ok_or_else(|| err("no account to allocate"))?;
                        if !acc.data.is_empty() || acc.owner != soltx::SYSTEM_PROGRAM {
                            return Err(err("AccountAlreadyInUse"));
                        }
                        acc.owner = owner;
                        acc.data = vec![0; space];
                    }
                    6 => {
                        let k = ix.accounts[0].key;
                        let authority: Pubkey = ix.data[4..36].try_into().unwrap();
                        let st = soltx::NonceState { authority, durable_nonce: super::sol::hash_of("opened", now), lamports_per_signature: 5_000 };
                        a.get_mut(&k).ok_or_else(|| err("no nonce account"))?.data = soltx::encode_nonce(&st);
                    }
                    t => return Err(err(&format!("system instruction {t}"))),
                }
                self.log.push(format!("system:{tag}"));
            } else {
                return Err(err(&format!("program {}", soltx::b58_encode(&ix.program))));
            }
        }
        Ok(())
    }
}

/// Install the world on a chain: landing a transaction executes it there (and a simulation runs it on copies).
pub fn install(chain: &Rc<RefCell<SolChain>>, world: &Rc<RefCell<SysWorld>>) {
    let w = world.clone();
    chain.borrow_mut().exec = Some(Box::new(move |ixs, _payer, accounts, now| {
        let mut next = w.borrow().clone();
        next.exec(ixs, accounts, now)?;
        *w.borrow_mut() = next;
        Ok(())
    }));
    let w = world.clone();
    chain.borrow_mut().sim = Some(Box::new(move |ixs, _payer, accounts, now| {
        let mut next = w.borrow().clone();
        let mut acc = accounts.clone();
        next.exec(ixs, &mut acc, now)
    }));
}

/// Relay's Solana depository as read on 2026-10-09 (`tests/fixtures/relay-sol/chain`): its state and its `ProgramData` header (rule S3),
/// and its SOL vault.
pub fn install_relay(chain: &mut SolChain) {
    let f = |n: &str| -> Value {
        serde_json::from_str(&std::fs::read_to_string(format!("{}/tests/fixtures/relay-sol/chain/{n}", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap()
    };
    let b64 = |v: &Value| -> Vec<u8> {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.decode(v["data"][0].as_str().or_else(|| v["value"]["data"][0].as_str()).unwrap()).unwrap()
    };
    let st = f("depository-account.json");
    let pd = f("programdata-head.json");
    chain.set_account(soltx::key(relaysol::RELAY_DEPOSITORY), soltx::key(relaysol::DEPOSITORY_PROGRAM), 1_628_640, b64(&st["result"]["value"]));
    chain.set_account(soltx::key(relaysol::PROGRAMDATA), soltx::key(relaysol::BPF_LOADER_UPGRADEABLE), 1_000_000_000, b64(&pd["result"]["value"]));
    chain.set_account(soltx::key(relaysol::VAULT), soltx::SYSTEM_PROGRAM, 1_000_000_000, vec![]);
}
