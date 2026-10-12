//! The Ethereum NFT side with its home on SOLANA, the BOOKS home (docs/v9/solana-migration.md §4; docs/v9/canister-only-solana.md §8,
//! decision D-l): the flows of `tests/ethnft.rs` run again with every home read from the books and every home step a book update, but
//! the advance (`M[i] → K_main`, signed by the collection's key on the money line) and the Relay deposit, LANDED by a fake Solana chain
//! that executes the System Program and Relay's depository (`fakerpc::sys`), every read at `finalized` through the fake provider pair
//! (`fakerpc::sol`, running the canister's own transform). Ethereum is the same fake world as `tests/ethnft.rs`'s. The Relay quotes are
//! the real Solana-origin ones captured 2026-10-09 (spike Q5: `sol-eth-20`, `eth-sol-0.5`); the floor's sales are the real receipts.
mod fakerpc;

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use etch_autobuyer::custodycap::{self, Book, CapPolicy};
use etch_autobuyer::dormancy::Dormancy;
use etch_autobuyer::ethflow::{self as ef, BPhase, EthIo, EthPin, EthPlan, EthState, OurReceipt};
use etch_autobuyer::ethnft::{self as en, EthCollection, EthNftConfig, LogV, ReceiptV, Seen};
use etch_autobuyer::evmtx::{keccak, Tx1559};
use etch_autobuyer::itemsink::Clock;
use etch_autobuyer::looper::EvmView;
use etch_autobuyer::outflow::Flow;
use etch_autobuyer::rates;
use etch_autobuyer::relaysol;
use etch_autobuyer::seaport::{self as sp, Word};
use etch_autobuyer::solhome::{self, SolHomeCfg, SolIo, SolLine};
use etch_autobuyer::solkey::SolSigner as _;
use etch_autobuyer::solrpc::{self, Account, Level, SigInfo};
use etch_autobuyer::solsend::{self, LifetimeKind, Line};
use etch_autobuyer::soltx::{self, Instruction, Pubkey};
use fakerpc::sol::{pair, FakeSigner, FakeSol, SolChain};
use fakerpc::sys::{self, SysWorld, ONE};
use serde_json::{json, Value};

fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

const E: u128 = 1_000_000_000_000_000_000;
const SALES: &str = include_str!("fixtures/seaport/milady-sales-receipts.json");
const FILL: &str = include_str!("fixtures/seaport/eth-milady-fill-26055691.json");
/// The real SOL → ETH quote of 2026-10-09 for 20 SOL, and the real ETH → SOL one for 0.5 ETH (spike Q5): their `user`, our Solana key;
/// their Ethereum recipient, our EVM address.
const SOL_ETH_20: &str = include_str!("fixtures/relay-sol/quotes/sol-eth-20.quote.json");
const ETH_SOL: &str = include_str!("fixtures/relay-sol/quotes/eth-sol-0.5.quote.json");
const ME_SOL: &str = "5ecsyZA5SVEuTByKoDCUivha8uzAgeQJmgFnnvokFF21";

/// Audit A5-03: the quote as Relay answers an ask whose `user` is our dedicated Relay key (the deposit's sender and depositor) and whose
/// refunds go to our Solana address: the fixture's instruction keys name the Relay key (the order, so its id, does not name the sender).
fn relay_quote(q: &str) -> Vec<u8> {
    let relay = soltx::b58_encode(&fakerpc::sol::relay_of(&soltx::pubkey(ME_SOL).unwrap()));
    let mut v: Value = serde_json::from_str(q).unwrap();
    for k in v["steps"][0]["items"][0]["data"]["instructions"][0]["keys"].as_array_mut().unwrap() {
        if k["pubkey"] == ME_SOL {
            k["pubkey"] = relay.clone().into();
        }
    }
    serde_json::to_vec(&v).unwrap()
}

const US: [u8; 20] = [0x9d, 0xb3, 0xfb, 0x11, 0x7e, 0xcf, 0xc8, 0xfc, 0x14, 0xca, 0x26, 0x3e, 0x40, 0x1a, 0x4c, 0x57, 0xa6, 0x91, 0x3d, 0x81];
/// The quotes' fetch time (their deadlines are this + 7 days).
const NOW: u64 = 1_791_565_570 + 5;
/// SOL/ETH at the quotes' own price: 20 SOL for 0.876567 ETH, 22.816 SOL an ETH.
const RATE: u64 = 22_816_000_000;
const MILADY: [u8; 20] = [0x5a, 0xf0, 0xd9, 0x82, 0x7e, 0x0c, 0x53, 0xe4, 0x79, 0x9b, 0xb2, 0x26, 0x65, 0x5a, 0x1d, 0xe1, 0x52, 0xa4, 0x25, 0xa5];
const ETH_PIN: u64 = 26_068_928;
/// The advance: 20 SOL for the quote and the Solana keep. The module's backing per net unit is exactly this (2 units, twice it), so
/// it is its `max_item_cost`, the advance's cap.
const ADVANCE: u64 = 20_000_000_000 + solhome::SOL_GAS_KEEP_LAMPORTS as u64;
/// The module's accepted floor: 19.5 SOL a unit (0.8547 ETH at the rate): the cap covers it on Ethereum with the margin (A2-M-1).
const ACCEPTED: u64 = 19_500_000_000;

struct World {
    now: u64,
    paused: bool,
    rate: Result<u64, String>,
    dormancy: Option<Dormancy>,
    // the home: a Solana chain, its two providers, our key
    chain: Rc<RefCell<SolChain>>,
    ps: Vec<FakeSol>,
    signer: FakeSigner,
    sys: Rc<RefCell<SysWorld>>,
    me: Pubkey,
    /// The canister-wide Solana line (phase B2), shared with any other flow of the same world.
    line: Rc<RefCell<SolLine>>,
    // Ethereum (tests/ethnft.rs's world)
    eth_balance: u128,
    eth_nonce: u64,
    finalized: u64,
    owners: HashMap<Word, [u8; 20]>,
    eth_signed: Vec<(Tx1559, [u8; 32])>,
    receipts: HashMap<[u8; 32], OurReceipt>,
    listing_total: u128,
    sale_receipts: HashMap<[u8; 32], ReceiptV>,
    our_logs: HashMap<[u8; 32], ReceiptV>,
    caps: CapPolicy,
    book: Book,
    outflow: Vec<(String, Flow, u128)>,
    /// Audit A5-02: the Bitcoin side as `EthIo` sees it (a conversion holding the Relay leg; a reimbursement owed).
    frog_busy: bool,
    frog_owed: bool,
    /// 2026-10-12: the side's chain (1, or Robinhood Chain's 4663) and its collection (Milady on 1, CASHCAT on 4663).
    evm: u64,
    coll: [u8; 20],
    /// On the 4663 side the fake judges a handed-in Seaport order as `nftnet::check` would at its first rules: the collection, and the
    /// offerer's EIP-712 signature over Seaport 1.6's domain on THIS chain (the chain id is in the domain).
    judge: bool,
}

impl World {
    /// W8: the world on the books home: the collection listed in the books (2 SOL-backed units, the module holding twice the advance),
    /// the module's SOL at the collection key's address; no program of Rook's on the chain.
    fn books() -> World {
        World::books_on(1, MILADY)
    }

    /// The books world of one side: `chain`'s collection `coll` listed in the books as an EthNft of that chain.
    fn books_on(evm: u64, coll: [u8; 20]) -> World {
        let me = soltx::pubkey(ME_SOL).unwrap();
        let chain = SolChain::new();
        chain.borrow_mut().unix = NOW;
        chain.borrow_mut().set_account(me, soltx::SYSTEM_PROGRAM, 5_000_000_000, vec![]);
        let sys = Rc::new(RefCell::new(SysWorld::default()));
        sys::install(&chain, &sys);
        sys::install_relay(&mut chain.borrow_mut());
        let ps = pair(&chain);
        let mut sale_receipts = HashMap::new();
        let v: Value = serde_json::from_str(&sales_of(coll)).unwrap();
        for (k, r) in v["receipts"].as_object().unwrap() {
            let h: [u8; 32] = hex::decode(&k[2..]).unwrap().try_into().unwrap();
            let mut r = en::receipt_of(r).unwrap().unwrap();
            // audit C3-02: the synthetic sales are Ethereum's blocks; on another chain they keep their place under the final depth,
            // which is that chain's (14,769 of 4663's 0.104 s blocks, not Ethereum's 128)
            r.block -= en::sale_blocks(evm).final_depth - en::sale_blocks(1).final_depth;
            sale_receipts.insert(h, r);
        }
        let w = World { now: NOW, paused: false, rate: Ok(RATE), dormancy: None, chain, ps, signer: FakeSigner::new(me), sys, me,
            line: Rc::new(RefCell::new(SolLine::default())),
            eth_balance: 10_000_000_000_000_000, eth_nonce: 0, finalized: ETH_PIN - 100, owners: HashMap::new(), eth_signed: vec![],
            receipts: HashMap::new(), listing_total: 45 * E / 100, sale_receipts, our_logs: HashMap::new(),
            caps: custodycap::stage0(&[custodycap::evm_key(evm, &coll)]), book: Book::default(), outflow: vec![], frog_busy: false,
            frog_owed: false, evm, coll, judge: evm != 1 };
        etch_autobuyer::desk::state::reset();
        etch_autobuyer::desk::state::with_mut(|s| {
            s.books.add_collection(1, etch_autobuyer::books::ListingKind::EthNft { chain_id: evm, collection: coll, custody: US }, [0xC0; 32],
                etch_autobuyer::books::default_params(20 * 1_000_000_000)).unwrap();
            let c = s.books.collections.get_mut(&1).unwrap();
            let m = &mut c.module;
            m.accepted = ACCEPTED;
            m.target = ACCEPTED;
            m.posted_at = NOW as i64 - 60;
            m.confidence_bps = 8_000;
            m.depth_items = 10;
            m.day_anchor = ACCEPTED;
            m.day_anchor_at = NOW as i64 - 60;
            m.last_good_at = NOW as i64 - 60;
            m.backing = 2 * ADVANCE;
            c.ledger.sol_backed = 2 * ONE;
        });
        w.chain.borrow_mut().set_account(books_module(), soltx::SYSTEM_PROGRAM, 2 * ADVANCE + sys::RENT, vec![]);
        w
    }

    /// The books collection, as the flow sees it.
    fn ledger(&self) -> etch_autobuyer::books::ledger::Ledger {
        etch_autobuyer::desk::state::with(|s| s.books.get(1).unwrap().ledger.clone())
    }
    fn bmodule(&self) -> etch_autobuyer::books::floor::Module {
        etch_autobuyer::desk::state::with(|s| s.books.get(1).unwrap().module.clone())
    }

    fn cfg(&self) -> EthNftConfig {
        EthNftConfig { chain_id: self.evm, relay_max_fee_bps: 300, deposits: false, floor: None, gas_pot_target_wei: None, logs_max_blocks: None,
            dormancy: self.dormancy.clone(), collections: vec![EthCollection { collection: format!("0x{}", hex::encode(self.coll)),
                wrapper: "books".into(), module: "books".into(), max_purchase_wei: 2 * E, max_premium_bps: 500, chain_id: None, token_bound: None, nft_value_bps: None }] }
    }

    fn lamports(&self) -> u64 {
        self.chain.borrow().accounts[&self.me].lamports
    }

    /// Land everything sent, at finalized (the fake chain executes it).
    fn land(&mut self) -> Vec<String> {
        self.chain.borrow_mut().process(Level::Finalized)
    }

    /// Mine an Ethereum transaction (tests/ethnft.rs's `mine_eth`).
    fn mine_eth(&mut self, i: usize, success: bool, block: u64) {
        let (tx, hash) = self.eth_signed[i].clone();
        let fee = 21_000 * 1_000_000_000;
        self.eth_balance -= fee + if success { tx.value } else { 0 };
        self.eth_nonce = self.eth_nonce.max(tx.nonce + 1);
        if success {
            if tx.to == sp::SEAPORT_1_6 {
                self.owners.insert(sp::word_u128(9_180), US);
            } else if tx.to == self.coll {
                let dest: [u8; 20] = tx.data[4 + 32 + 12..4 + 64].try_into().unwrap();
                self.owners.insert(tx.data[4 + 64..4 + 96].try_into().unwrap(), dest);
            }
        }
        if tx.data.len() == 4 + 96 && tx.data[..4] == en::TRANSFER_FROM {
            let logs = if success { vec![LogV { address: tx.to, topics: vec![en::TRANSFER, tx.data[4..36].try_into().unwrap(),
                tx.data[36..68].try_into().unwrap(), tx.data[68..100].try_into().unwrap()], data: vec![] }] } else { vec![] };
            self.our_logs.insert(hash, ReceiptV { block, success, logs });
        }
        self.receipts.insert(hash, OurReceipt { block, success, fee });
    }
}

impl EthIo for World {
    fn frog_relay_busy(&self) -> bool { self.frog_busy }
    fn frog_reimbursement_owed(&self) -> bool { self.frog_owed }
    fn now(&self) -> u64 { self.now }
    fn home_chain(&self) -> u64 { relaysol::RELAY_SOLANA }
    fn eth_chain(&self) -> u64 { self.evm }
    fn config(&self) -> Option<EthNftConfig> { Some(self.cfg()) }
    fn paused(&self) -> bool { self.paused }
    fn eth_max_fee(&self) -> u128 { 50_000_000_000 }
    fn save(&mut self, _st: &EthState) {}
    fn cycles_balance(&self) -> Option<u128> { Some(100_000_000_000_000) }
    fn with_caps<R>(&mut self, f: impl FnOnce(&CapPolicy, &mut Book) -> R) -> R { f(&self.caps, &mut self.book) }
    fn outflow_check(&mut self, _key: &str, _c: &str, _flow: Flow, _amount: u128) -> Result<(), String> { Ok(()) }
    fn outflow_record(&mut self, key: &str, _c: &str, flow: Flow, amount: u128) { self.outflow.push((key.into(), flow, amount)); }

    async fn our_evm(&mut self) -> Result<[u8; 20], String> { Ok(US) }
    // the home is Solana: no EVM read, signature or nonce is ever asked
    async fn home_read(&mut self, to: [u8; 20], _d: &[u8]) -> Result<Vec<u8>, String> { panic!("an EVM home read of 0x{} on the Solana home", hex::encode(to)) }
    async fn home_clock(&mut self) -> Result<Clock, String> {
        let (time, slot) = solrpc::finalized_clock(&mut self.ps).await?;
        Ok(Clock { time, parent_block: slot })
    }
    async fn home_balance(&mut self) -> Result<u128, String> { Ok(solrpc::balance(&mut self.ps, &self.me).await? as u128) }
    async fn home_sign(&mut self, _to: [u8; 20], _v: u128, _d: &[u8]) -> Result<(Vec<u8>, [u8; 32], u64), String> { panic!("an EVM home signature") }
    async fn home_broadcast(&mut self, _raw: &[u8]) { panic!("an EVM home broadcast") }
    async fn home_seen(&mut self, _h: [u8; 32]) -> EvmView { panic!("an EVM home status") }
    async fn home_nonce(&mut self) -> Result<u64, String> { panic!("an EVM home nonce") }
    async fn home_finalized_time(&mut self) -> Result<u64, String> { Ok(solrpc::finalized_clock(&mut self.ps).await?.0) }

    async fn eth_pin(&mut self) -> Result<EthPin, String> { Ok(EthPin { number: ETH_PIN, timestamp: self.now, base_fee: 100_000_000 }) }
    async fn eth_finalized(&mut self) -> Result<u64, String> { Ok(self.finalized) }
    async fn eth_read(&mut self, to: [u8; 20], d: &[u8]) -> Result<Vec<u8>, String> {
        if to == etch_autobuyer::relayconv::RELAY_DEPOSITORY_4663 && d[..4] == etch_autobuyer::relayconv::ALLOCATOR {
            return Ok(sp::word_addr(&etch_autobuyer::relayconv::RELAY_ALLOCATOR_4663).to_vec());
        }
        assert_eq!(to, self.coll, "an Ethereum read of 0x{}", hex::encode(to));
        let t: Word = d[4..36].try_into().unwrap();
        Ok(sp::word_addr(self.owners.get(&t).unwrap_or(&[0x0e; 20])).to_vec())
    }
    async fn eth_balance(&mut self) -> Result<u128, String> { Ok(self.eth_balance) }
    async fn eth_nonce(&mut self) -> Result<u64, String> { Ok(self.eth_nonce) }
    async fn eth_receipt(&mut self, h: [u8; 32]) -> Result<Option<OurReceipt>, String> { Ok(self.receipts.get(&h).copied()) }
    async fn eth_plan_buy(&mut self, order: &str, collection: [u8; 20], cap: u128) -> Result<EthPlan, String> {
        assert_eq!(collection, self.coll);
        if self.judge {
            // `nftnet::check` on this chain: the order sells this collection, and its offerer signed it over THIS chain's Seaport 1.6
            // domain (an order signed for Ethereum is not the offerer's on 4663)
            let o = sp::decode(&hex::decode(order.trim_start_matches("0x")).unwrap())?;
            if o.offer_token != collection {
                return Err(format!("refused (wrong collection): the order sells 0x{}", hex::encode(o.offer_token)));
            }
            if sp::signed_by_offerer(&o, self.evm, &sp::SEAPORT_1_6, [0; 32])? != true {
                return Err("refused (signature): not checkable".into());
            }
            self.listing_total = o.total().ok_or("no total")?;
        }
        if self.listing_total > cap {
            return Err(format!("refused (over cap): the order costs {}; the cap is {cap}", self.listing_total));
        }
        Ok(EthPlan { to: sp::SEAPORT_1_6, value: self.listing_total, data: vec![0xfb, 0x0f, 0x3e, 0xe1], gas: 200_000, max_fee: 1_000_000_000,
            priority_fee: 100_000_000, nonce: self.eth_nonce, token: sp::word_u128(9_180), total: self.listing_total })
    }
    async fn eth_plan_transfer(&mut self, collection: [u8; 20], token: Word, dest: [u8; 20]) -> Result<EthPlan, String> {
        Ok(EthPlan { to: collection, value: 0, data: en::transfer_from_call(&US, &dest, token), gas: 60_000, max_fee: 1_000_000_000,
            priority_fee: 100_000_000, nonce: self.eth_nonce, token, total: 0 })
    }
    async fn eth_sign(&mut self, tx: &Tx1559) -> Result<Vec<u8>, String> {
        let raw = [&tx.nonce.to_be_bytes()[..], &tx.to, &tx.value.to_be_bytes(), &tx.max_fee.to_be_bytes(), &tx.data].concat();
        self.eth_signed.push((tx.clone(), keccak(&raw)));
        Ok(raw)
    }
    async fn eth_broadcast(&mut self, _raw: &[u8]) {}
    async fn eth_listing(&mut self, s: &str, _c: [u8; 20]) -> Result<Seen, String> {
        let (o, p) = s.split_once(':').ok_or("not a fake listing")?;
        Ok(Seen { offerer: [o.parse::<u8>().unwrap(); 20], token: sp::word_u128(o.parse().unwrap()), price: p.parse().unwrap() })
    }
    async fn eth_sale_receipt(&mut self, h: [u8; 32]) -> Result<Option<ReceiptV>, String> {
        if let Some(r) = self.our_logs.get(&h) {
            return Ok(Some(r.clone()));
        }
        Ok(self.sale_receipts.get(&h).cloned())
    }
    async fn eth_transfer_txs(&mut self, collection: [u8; 20], from: u64, to: u64) -> Result<Vec<([u8; 32], u64)>, String> {
        let mut v: Vec<([u8; 32], u64)> = self.sale_receipts.iter().filter(|(_, r)| r.block >= from && r.block <= to && r.logs.iter().any(|l|
            l.address == collection && l.topics.len() == 4 && l.topics[0] == en::TRANSFER && l.topics[1] != l.topics[2] && l.topics[1] != [0; 32]))
            .map(|(h, r)| (*h, r.block)).collect();
        v.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        Ok(v)
    }
    async fn eth_plan_call(&mut self, to: [u8; 20], value: u128, data: &[u8]) -> Result<EthPlan, String> {
        Ok(EthPlan { to, value, data: data.to_vec(), gas: 60_000, max_fee: 1_000_000_000, priority_fee: 100_000_000, nonce: self.eth_nonce,
            token: [0; 32], total: value })
    }
    async fn eth_deposit_tx(&mut self, _h: [u8; 32]) -> Result<Option<([u8; 20], [u8; 20], Vec<u8>, ReceiptV)>, String> { Ok(None) }
}

impl SolIo for World {
    fn sol_home(&self) -> Option<SolHomeCfg> {
        Some(SolHomeCfg { lifetime: LifetimeKind::DurableNonce, nonce_seed: "rook-home-0".into() })
    }
    async fn sol_me(&mut self) -> Result<Pubkey, String> { Ok(self.me) }
    async fn sol_key_of(&mut self, p: &etch_autobuyer::solkey::KeyPath) -> Result<Pubkey, String> { self.signer.key(p).await }
    async fn sol_accounts(&mut self, keys: &[Pubkey], max_data: usize) -> Result<Vec<Option<Account>>, String> {
        Ok(solrpc::accounts(&mut self.ps, keys, max_data).await?)
    }
    async fn sol_account_head(&mut self, key: &Pubkey, len: usize) -> Result<Option<Account>, String> {
        Ok(solrpc::account_slice(&mut self.ps, key, 0, len).await?)
    }
    async fn sol_program_accounts(&mut self, program: &Pubkey, size: usize, offset: usize, bytes: &[u8], max: usize) -> Result<Vec<(Pubkey, Account)>, String> {
        Ok(solrpc::program_accounts(&mut self.ps, program, size, offset, bytes, max, size).await?)
    }
    async fn sol_rent(&mut self, space: u64) -> Result<u64, String> { Ok(solrpc::rent_exempt(&mut self.ps, space).await?) }
    async fn sol_relay(&mut self) -> Result<Pubkey, String> { Ok(self.signer.relay) }
    async fn sol_balance_of(&mut self, key: &Pubkey) -> Result<u128, String> { Ok(solrpc::balance(&mut self.ps, key).await? as u128) }
    async fn sol_rate(&mut self) -> Result<u64, String> { self.rate.clone() }
    async fn sol_prepare(&mut self, line: &mut Line, kind: LifetimeKind, intent: u64, what: &str, ixs: Vec<Instruction>) -> Result<(), String> {
        solsend::prepare(line, kind, intent, what, ixs, &mut self.ps, &mut self.signer).await
    }
    async fn sol_prepare_keys(&mut self, line: &mut Line, kind: LifetimeKind, intent: u64, what: &str, ixs: Vec<Instruction>,
        keys: Vec<etch_autobuyer::solkey::KeyPath>) -> Result<(), String> {
        solsend::prepare_keys(line, kind, intent, what, ixs, keys, &mut self.ps, &mut self.signer).await
    }
    async fn sol_tick(&mut self, line: &mut Line) -> Result<solsend::Step, String> {
        Ok(solsend::tick(line, &mut self.ps, &mut self.signer).await?)
    }
    async fn sol_signatures(&mut self, until: &str, limit: usize) -> Result<Vec<SigInfo>, String> {
        Ok(solrpc::signatures_since(&mut self.ps, &self.me, until, limit).await?)
    }
    async fn sol_transaction(&mut self, sig: &str) -> Result<Option<Value>, String> {
        Ok(solrpc::transaction(&mut self.ps, sig, solhome::TX_BYTES).await?)
    }

    async fn sol_balance(&mut self) -> Result<u128, String> { Ok(solrpc::balance(&mut self.ps, &self.me).await? as u128) }
    async fn sol_clock(&mut self) -> Result<(u64, u64), String> { Ok(solrpc::finalized_clock(&mut self.ps).await?) }
    async fn sol_program_slices(&mut self, program: &Pubkey, size: usize, memcmps: &[(usize, Vec<u8>)], slice: (usize, usize), max: usize)
        -> Result<Vec<(Pubkey, Pubkey, Vec<u8>)>, String> {
        Ok(solrpc::program_slices(&mut self.ps, program, size, memcmps, slice, max).await?)
    }
    async fn sol_genesis(&mut self) -> Result<String, String> { Ok(solrpc::genesis_hash(&mut self.ps).await?) }
    fn sol_line(&mut self) -> SolLine { self.line.borrow().clone() }
    fn sol_line_save(&mut self, l: &SolLine) { *self.line.borrow_mut() = l.clone(); }
}

/// W8: the module's address on the books home, `K_coll[1]`.
fn books_module() -> Pubkey {
    fakerpc::sol::derived_key(&etch_autobuyer::solkey::KeyPath::Coll(1))
}

fn tick(w: &mut World, st: &mut EthState) -> Vec<String> {
    w.chain.borrow_mut().unix = w.now;
    let lines = block_on(ef::tick(w, st, false));
    for l in &lines {
        println!("{l}");
    }
    lines
}

fn phase(st: &EthState) -> BPhase {
    st.buys.last().unwrap().phase.clone()
}

fn sales() -> Vec<[u8; 32]> {
    let v: Value = serde_json::from_str(SALES).unwrap();
    v["receipts"].as_object().unwrap().keys().map(|k| hex::decode(&k[2..]).unwrap().try_into().unwrap()).collect()
}

fn fill_hex() -> String {
    let v: Value = serde_json::from_str(FILL).unwrap();
    v["input"].as_str().unwrap().to_string()
}

fn evidence(w: &World, st: &mut EthState) {
    ef::put_evidence(st, &w.cfg(), MILADY, vec![format!("1:{}", 92 * E / 100), format!("2:{}", 93 * E / 100), format!("3:{}", 95 * E / 100)],
        sales(), w.now).unwrap();
}

/// What the fake chain executed, by name (`system:<tag>`, `deposit_native`): never a program instruction of Rook's.
fn sys_log(w: &World) -> Vec<String> {
    w.sys.borrow().log.clone()
}

/// Audit A5-02: a way back waiting for its quote holds the Relay leg (the frog side reads `relay_busy`), and its deposit is never signed
/// while a Bitcoin-side conversion holds the leg (its watch reads the same Solana balance Relay's ETH → SOL payout lands in).
#[test]
fn a5_02_the_way_backs_deposit_waits_while_a_bitcoin_side_conversion_holds_the_relay_leg() {
    let mut w = World::books();
    let mut st = EthState::default();
    books_to_arrival(&mut w, &mut st);
    w.now += ef::ARRIVED_BACK_SECS + 1;
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st), BPhase::BackFunded { .. }), "{:?}", phase(&st));
    assert!(st.relay_busy(), "a way back waiting for its quote holds the leg (A5-02)");
    st.buys[0].ask.as_mut().unwrap().amount_wei = 500_000_000_000_000_000;
    let q: Value = serde_json::from_str(ETH_SOL).unwrap();
    w.now = q["protocol"]["v2"]["orderData"]["output"]["deadline"].as_u64().unwrap() - 7 * 86_400 + 5;
    st.buys[0].phase = BPhase::BackFunded { wei: 870_000_000_000_000_000, since: w.now };
    // a frog conversion took the leg meanwhile: the quote is judged nowhere, nothing is signed
    w.frog_busy = true;
    let signed = w.eth_signed.len();
    ef::put_quote(&mut st, 0, ETH_SOL.as_bytes().to_vec(), w.now).unwrap();
    let l = tick(&mut w, &mut st);
    assert!(matches!(phase(&st), BPhase::BackFunded { .. }), "{:?} {l:?}", phase(&st));
    assert_eq!(w.eth_signed.len(), signed, "no deposit signed: {l:?}");
    assert!(st.buys[0].note.contains("Bitcoin-side conversion holds the Relay leg"), "{}", st.buys[0].note);
    // the leg free again: the next quote goes
    w.frog_busy = false;
    ef::put_quote(&mut st, 0, ETH_SOL.as_bytes().to_vec(), w.now).unwrap();
    let l = tick(&mut w, &mut st);
    assert!(matches!(phase(&st), BPhase::BackSending { .. }), "{:?} {l:?}", phase(&st));
}

/// Audit A5-02, the symmetric rule: on the Solana home no new buy opens a Relay watch while a reimbursement of the Bitcoin side (a frog's
/// `item_arrived`, which pays our Solana address) is owed.
#[test]
fn a5_02_no_new_buy_opens_a_watch_while_a_frog_reimbursement_is_owed() {
    let mut w = World::books();
    let mut st = EthState::default();
    w.frog_owed = true;
    let l = tick(&mut w, &mut st);
    assert!(st.buys.is_empty(), "{:?} {l:?}", st.buys);
    assert!(l.iter().any(|x| x.contains("reimbursement of the Bitcoin side")), "{l:?}");
    w.frog_owed = false;
    books_to_funded(&mut w, &mut st);
}

/// Decision D-l: on the Solana home every collection's wrapper and module is `"books"` (the canister keeps the books); a configuration
/// naming addresses is refused on it, and `"books"` is refused on the EVM home; holder deposits stay off.
#[test]
fn books_a_collection_is_books_on_the_solana_home_and_deposits_cannot_be_switched_on() {
    let w = World::books();
    let mut cfg = w.cfg();
    assert!(en::validate_home(&cfg, true).is_ok());
    assert!(en::validate_home(&cfg, false).unwrap_err().contains("books"), "a books collection on the EVM home");
    let c = cfg.cols().unwrap()[0];
    assert!(c.books && c.wrapper != [0; 20] && c.module != [0; 20] && c.wrapper != c.module, "derived record keys");
    cfg.deposits = true;
    assert!(en::validate_home(&cfg, true).unwrap_err().contains("OFF on the Solana home"));
    // W15: the W10 install argument's shape, every Ethereum collection `wrapper = module = "books"`, is what passes here
    assert_eq!((cfg.collections[0].wrapper.as_str(), cfg.collections[0].module.as_str()), ("books", "books"));
    let mut evm = w.cfg();
    evm.collections[0].wrapper = "0x00000000000000000000000000000000000000aa".into();
    evm.collections[0].module = "0x00000000000000000000000000000000000000bb".into();
    assert!(en::validate_home(&evm, true).unwrap_err().contains("D-l"), "addresses on the Solana home: refused");
    assert!(en::validate_home(&evm, false).is_ok());
    let mut mixed = w.cfg();
    mixed.collections[0].module = "0x00000000000000000000000000000000000000bb".into();
    assert!(mixed.cols().unwrap_err().contains("both are \"books\" or neither"));
}

// ---- the dormancy on Solana: the wake desk's request, the wake, the post that served it, recorded on the desk -------------------------

#[test]
fn books_the_home_has_no_wake_desk_a_configured_one_is_refused_at_install_and_none_is_fine() {
    let d = Dormancy::launch(Some(soltx::b58_encode(&[7u8; 32])));
    let e = etch_autobuyer::canister::solana_desk_ok(&d).unwrap_err();
    assert!(e.contains("the books home has no wake desk") && e.contains("O-11"), "{e}");
    assert_eq!(etch_autobuyer::canister::solana_desk_ok(&Dormancy::launch(None)), Ok(()));
}

// ---- audit 2026-10 fixes (docs/v9/audit-2026-10/a4-*.md, a5-*.md "## Fixes") -------------------------------------------------------

/// A4-02: a transfer to the line's nonce address before it opens (a System-owned account, no data) does not block the line: it is
/// opened in place (`AllocateWithSeed` + a top-up + `InitializeNonceAccount`), signed by us alone, and the advance follows.
#[test]
fn fix_a4_02_a_squatted_nonce_address_is_opened_in_place() {
    let mut w = World::books();
    let mut st = EthState::default();
    let nonce = solsend::nonce_account_of(&w.me, "rook-home-0").unwrap();
    w.chain.borrow_mut().set_account(nonce, soltx::SYSTEM_PROGRAM, 890_880, vec![]);
    let l = tick(&mut w, &mut st);
    assert!(l.iter().any(|x| x.contains("opening its durable-nonce account")), "{l:?}");
    assert_eq!(w.land().len(), 1, "the opening lands");
    assert_eq!(sys_log(&w), vec!["system:9", "system:2", "system:6"], "allocate in place, top up to the rent, initialize");
    let rent = 6_960 * (128 + soltx::NONCE_ACCOUNT_SPACE);
    {
        let c = w.chain.borrow();
        assert_eq!(c.nonce_of(&nonce).authority, w.me, "ours");
        assert_eq!(c.accounts[&nonce].lamports, rent, "topped up to exactly the rent-exempt amount");
    }
    let l = tick(&mut w, &mut st);
    assert!(l.iter().any(|x| x.contains("is open")), "{l:?}");
    assert_eq!(w.line.borrow().line.nonce_account, Some(nonce));
    assert_eq!(phase(&st), BPhase::Advancing, "the advance follows: {l:?}");
    // a nonce account there with ANOTHER authority is still refused for the operator
    let mut w = World::books();
    let mut st = EthState::default();
    w.chain.borrow_mut().set_nonce(nonce, [0x99; 32]);
    let l = tick(&mut w, &mut st);
    assert!(l.iter().any(|x| x.contains("is not an initialized nonce account of ours")), "{l:?}");
}

/// A5-03: the deposit fails on chain: the Relay key (funded with the deposit and its fee) is swept back to our key BEFORE the flow is
/// told, so no balance watch reads it as a refund; nothing is left at the Relay key.
#[test]
fn fix_a5_03_a_failed_deposit_is_swept_back_from_the_relay_key_before_its_flow_is_told() {
    let mut w = World::books();
    let mut st = EthState::default();
    books_to_funded(&mut w, &mut st);
    let before = w.lamports();
    ef::put_quote(&mut st, 0, relay_quote(SOL_ETH_20), w.now).unwrap();
    tick(&mut w, &mut st);
    assert_eq!(w.land().len(), 1, "the funding lands");
    let fund = 20_000_000_000 + solsend::SIGNATURE_FEE_LAMPORTS;
    assert_eq!(w.chain.borrow().accounts[&w.signer.relay].lamports, fund, "exactly the deposit and its fee");
    tick(&mut w, &mut st); // the deposit is signed by the Relay key
    w.chain.borrow_mut().fail_next = Some(json!({ "InstructionError": [0, { "Custom": 1 }] }));
    assert_eq!(w.land().len(), 1, "the deposit lands, failed");
    let l = tick(&mut w, &mut st);
    assert!(l.iter().any(|x| x.contains("swept back before its flow is told")), "{l:?}");
    assert!(matches!(phase(&st), BPhase::Depositing { .. }), "the flow is not told yet: {:?}", phase(&st));
    assert_eq!(w.land().len(), 1, "the sweep lands");
    let l = tick(&mut w, &mut st);
    assert!(l.iter().any(|x| x.contains("the Relay key is empty")), "{l:?}");
    assert!(!matches!(phase(&st), BPhase::Depositing { .. } | BPhase::Bridging { .. }), "taken back: {:?}", phase(&st));
    assert_eq!(w.chain.borrow().accounts.get(&w.signer.relay).map_or(0, |a| a.lamports), 0, "nothing left at the Relay key");
    // the fake charges no fee for a failed landing (the deposit's): the funding's and the sweep's
    assert_eq!(w.lamports(), before - 2 * fakerpc::sol::FEE_LAMPORTS, "back but the fees (funding, sweep)");
    assert!(w.sys.borrow().deposits.is_empty(), "Relay took nothing");
    // the Relay key never signed anything but its deposit and its sweep, and never with our key beside it
    assert!(w.line.borrow().line.done.iter().all(|d| !d.what.is_empty()));
}

/// A5-03: lamports already at the Relay key (anyone can send some) are swept back first; that deposit is not made (its step taken back
/// once the key is empty), so a deposit always starts from an empty key funded with exactly what it needs.
#[test]
fn fix_a5_03_a_relay_key_holding_lamports_is_swept_back_before_any_deposit() {
    let mut w = World::books();
    let mut st = EthState::default();
    books_to_funded(&mut w, &mut st);
    let relay = w.signer.relay;
    w.chain.borrow_mut().set_account(relay, soltx::SYSTEM_PROGRAM, 1_000_000_000, vec![]);
    let before = w.lamports();
    ef::put_quote(&mut st, 0, relay_quote(SOL_ETH_20), w.now).unwrap();
    let l = tick(&mut w, &mut st);
    assert!(l.iter().any(|x| x.contains("swept back first")), "{l:?}");
    assert_eq!(w.land().len(), 1, "the sweep lands");
    let l = tick(&mut w, &mut st);
    assert!(l.iter().any(|x| x.contains("never landed")), "{l:?}");
    assert!(matches!(phase(&st), BPhase::Funded { .. }), "the deposit was not made: {:?}", phase(&st));
    assert_eq!(w.chain.borrow().accounts.get(&relay).map_or(0, |a| a.lamports), 0);
    assert_eq!(w.lamports(), before + 1_000_000_000 - fakerpc::sol::FEE_LAMPORTS, "all of it back but the sweep's fee (paid by the Relay key)");
    assert!(w.sys.borrow().deposits.is_empty());
}

/// A5-07: a handed-in quote is not consumed (nor any read paid for) while the line is another flow's turn: it waits, then is used.
#[test]
fn fix_a5_07_a_quote_waits_unconsumed_while_the_line_is_not_this_sides_turn() {
    let mut w = World::books();
    let mut st = EthState::default();
    books_to_funded(&mut w, &mut st);
    w.line.borrow_mut().queue = vec![(solhome::LineOwner::Frog, w.now)];
    ef::put_quote(&mut st, 0, relay_quote(SOL_ETH_20), w.now).unwrap();
    let sent = w.chain.borrow().sent.len();
    let l = tick(&mut w, &mut st);
    assert!(l.iter().any(|x| x.contains("the quote waits (kept)")), "{l:?}");
    assert_eq!(st.inbox.quotes.len(), 1, "the quote is kept");
    assert!(st.used_orders.is_empty(), "no order burned");
    assert_eq!(w.chain.borrow().sent.len(), sent, "nothing signed");
    assert!(matches!(phase(&st), BPhase::Funded { .. }));
    w.line.borrow_mut().queue.clear();
    let l = tick(&mut w, &mut st);
    assert!(matches!(phase(&st), BPhase::Depositing { .. }), "{l:?}");
}
// =====================================================================================================================================
// W8 (docs/v9/canister-only-solana.md §8): the same flow on the BOOKS home. The advance is a transfer `M[i] → K_main` signed by the
// collection's key (the books advanced first); the floor post, the item's mint and the settlement are book updates, final at once; the
// Relay legs are unchanged.
// =====================================================================================================================================

fn books_to_funded(w: &mut World, st: &mut EthState) {
    let before = w.lamports();
    let module_before = w.chain.borrow().accounts[&books_module()].lamports;
    let l = tick(w, st);
    assert!(l.iter().any(|x| x.contains("opening its durable-nonce account")), "{l:?}");
    assert_eq!(w.land().len(), 1, "the opening lands");
    let l = tick(w, st);
    assert_eq!(phase(st), BPhase::Advancing, "{l:?}");
    assert_eq!(w.bmodule().transit_units, ONE, "the books marked the unit in transit BEFORE the signature");
    assert_eq!(w.bmodule().advanced, ADVANCE);
    assert_eq!(w.land().len(), 1, "the advance (a system transfer from the module) lands");
    tick(w, st);
    let BPhase::Funded { wei, .. } = phase(st) else { panic!("{:?}", phase(st)) };
    assert_eq!(wei, 20_000_000_000, "the advance less the Solana keep, in lamports");
    assert_eq!(st.buys[0].advanced, ADVANCE as u128);
    let rent = 6_960 * (128 + soltx::NONCE_ACCOUNT_SPACE);
    assert_eq!(w.lamports(), before + ADVANCE - 2 * fakerpc::sol::FEE_LAMPORTS - rent, "the module paid us the advance; we paid two fees and the nonce's rent");
    assert_eq!(w.chain.borrow().accounts[&books_module()].lamports, module_before - ADVANCE, "paid from the collection key's account");
    assert!(sys_log(w).iter().all(|x| x.starts_with("system:")), "no program instruction: {:?}", sys_log(w));
}

fn books_to_arrival(w: &mut World, st: &mut EthState) {
    books_to_funded(w, st);
    ef::put_quote(st, 0, relay_quote(SOL_ETH_20), w.now).unwrap();
    let l = tick(w, st);
    assert!(matches!(phase(st), BPhase::Depositing { .. }), "{l:?}");
    assert_eq!(w.land().len(), 1, "the funding of the Relay key lands");
    tick(w, st);
    assert_eq!(w.land().len(), 1, "the deposit lands");
    tick(w, st);
    let BPhase::Bridging { value, min_out, .. } = phase(st) else { panic!("{:?}", phase(st)) };
    assert_eq!((value, min_out), (20_000_000_000, 859_035_589_742_793_043));
    w.eth_balance += 870_000_000_000_000_000;
    tick(w, st);
    let BPhase::Arrived { wei, .. } = phase(st) else { panic!("{:?}", phase(st)) };
    assert_eq!(wei, 870_000_000_000_000_000);
}

#[test]
fn books_a_buy_end_to_end_advance_relay_sol_to_eth_seaport_mint_item_settle_in_the_books() {
    let mut w = World::books();
    let mut st = EthState::default();
    books_to_arrival(&mut w, &mut st);
    evidence(&w, &mut st);
    ef::put_listing(&mut st, 0, fill_hex(), w.now).unwrap();
    let l = tick(&mut w, &mut st);
    let r = st.reading(&MILADY).unwrap().clone();
    let BPhase::Buying { total, .. } = st.buys[0].phase.clone() else { panic!("{:?}", st.buys[0].phase.clone()) };
    assert_eq!(total, 45 * E / 100);
    // the floor post is a book update: our reading in lamports, clamped into the module's bounds, final at once (nothing to land)
    let raw = rates::wei_to_lamports(r.post.price, RATE);
    assert!(l.iter().any(|x| x.contains("post_floor(") && x.contains(&format!("clamped from {raw}")) && x.contains("floor posted in the books")), "{l:?}");
    assert_eq!(w.land().len(), 0, "no transaction for a post");
    let m = w.bmodule();
    assert!(m.accepted > ACCEPTED && m.accepted == m.target && m.accepted < raw as u64, "clamped to the module's hourly move: {}", m.accepted);
    tick(&mut w, &mut st);
    w.mine_eth(0, true, ETH_PIN - 10);
    tick(&mut w, &mut st);
    assert!(matches!(st.buys[0].phase.clone(), BPhase::Bought { .. }));
    w.finalized = ETH_PIN;
    // mined, ours, final → the mint is a book update (final at once; the flow reads its landing next tick), then the settlement
    let l = tick(&mut w, &mut st);
    assert!(l.iter().any(|x| x.contains("minted to the module in the books")), "{l:?}");
    assert!(matches!(st.buys[0].phase.clone(), BPhase::Minting { .. }), "{:?}", st.buys[0].phase.clone());
    let ld = w.ledger();
    assert_eq!(ld.items.get(&0).map(|x| x.key), Some(sp::word_u128(9_180)), "the item is the bought token, in the books already");
    assert_eq!((ld.held, ld.module_units), (1, ONE), "its unit is at the module");
    let l = tick(&mut w, &mut st); // minted → settle, a book update
    assert!(l.iter().any(|x| x.contains("settled at") && x.contains("in the books")), "{l:?}");
    assert!(matches!(st.buys[0].phase.clone(), BPhase::Settling { .. }), "{:?}", st.buys[0].phase.clone());
    let l = tick(&mut w, &mut st);
    let BPhase::Settled { cost, .. } = st.buys[0].phase.clone() else { panic!("{:?} {l:?}", st.buys[0].phase.clone()) };
    let want = en::cost_of(ADVANCE as u128, 45 * E / 100, 870_000_000_000_000_000).unwrap();
    assert_eq!(cost, want, "the item's share of the advance, in lamports");
    let (m, ld) = (w.bmodule(), w.ledger());
    assert_eq!((m.transit_units, m.items_arrived, ld.module_units, ld.sol_backed, ld.held), (0, 1, 0, ONE, 1), "settled: the unit is item-backed");
    assert_eq!(w.land().len(), 0, "no transaction for the mint or the settlement");
    assert!(sys_log(&w).iter().all(|x| x.starts_with("system:") || x == "deposit_native"), "no program instruction: {:?}", sys_log(&w));
    assert!(w.book.contains(&format!("{}:{}", custodycap::evm_key(1, &MILADY), hex::encode(sp::word_u128(9_180)))));
    assert!(st.pending.is_none());
}

#[test]
fn books_an_advance_that_never_lands_is_taken_back_in_the_books_and_advanced_again() {
    let mut w = World::books();
    let mut st = EthState::default();
    tick(&mut w, &mut st);
    w.land();
    let l = tick(&mut w, &mut st);
    assert_eq!(phase(&st), BPhase::Advancing, "{l:?}");
    assert_eq!((w.bmodule().transit_units, w.bmodule().advanced), (ONE, ADVANCE), "advanced in the books before the signature");
    // something else moves the nonce: the saved advance can never land; the operator clears the incident; the step is taken back
    let nonce = solsend::nonce_account_of(&w.me, "rook-home-0").unwrap();
    w.chain.borrow_mut().move_nonce(&nonce);
    let l = tick(&mut w, &mut st);
    assert!(l.iter().any(|x| x.contains("INCIDENT")), "{l:?}");
    assert_eq!((w.bmodule().transit_units, w.bmodule().advanced), (ONE, ADVANCE), "still out: the books hold the advance");
    solhome::clear_incident(&mut w.line.borrow_mut(), false).unwrap();
    let l = tick(&mut w, &mut st);
    assert!(l.iter().any(|x| x.contains("abandoned after an incident")), "{l:?}");
    // taken back in the books, then advanced again this tick (signed on the moved nonce)
    assert_eq!(phase(&st), BPhase::Advancing, "signed again: {l:?}");
    assert_eq!((w.bmodule().transit_units, w.bmodule().advanced), (ONE, ADVANCE), "taken back and advanced once more: not twice");
    assert_eq!(w.land().len(), 1);
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st), BPhase::Funded { .. }), "{:?}", phase(&st));
    assert_eq!(w.chain.borrow().accounts[&books_module()].lamports, 2 * ADVANCE + sys::RENT - ADVANCE, "the module paid one advance");
}

/// Audit C3-03 (the canister's half): a funded buy that no Relay quote reaches (a runner that cannot quote its chain's leg) holds the
/// one Relay leg for `FUNDED_QUOTE_SECS` at most, then its advance goes back to its module in the books and the leg is free.
#[test]
fn fixed_c3_03_a_funded_buy_no_quote_reaches_goes_back_to_its_module_and_frees_the_relay_leg() {
    let mut w = World::books();
    let mut st = EthState::default();
    books_to_funded(&mut w, &mut st);
    w.now += ef::FUNDED_QUOTE_SECS - 1;
    let l = tick(&mut w, &mut st);
    assert!(matches!(phase(&st), BPhase::Funded { .. }), "still waiting: {l:?}");
    assert!(st.relay_busy());
    w.now += 1;
    let l = tick(&mut w, &mut st);
    assert!(l.iter().any(|x| x.contains("no Relay quote was judged") && x.contains("C3-03")), "{l:?}");
    assert_eq!(phase(&st), BPhase::Returning { wei: 20_000_000_000 }, "{l:?}");
    let mut l = Vec::new();
    for _ in 0..4 {
        l.extend(tick(&mut w, &mut st));
        w.land();
        if matches!(phase(&st), BPhase::Returned { .. }) {
            break;
        }
    }
    assert!(matches!(phase(&st), BPhase::Returned { .. }), "{:?} {l:?}", phase(&st));
    assert!(!st.relay_busy(), "the Relay leg is free");
    assert_eq!(w.bmodule().transit_units, 0, "the books took the advance back");
}

// =====================================================================================================================================
// 2026-10-12: Robinhood Chain NFTs (docs/v9/robinhood-everything.md §3). A collection of Robinhood Chain (4663) is an EthNft listing of
// that chain on the books home: its side runs the same flow on 4663 (its reads, its Seaport 1.6 fill, its gas pot, its way back, its
// deliveries), the SOL leg pays out on "robinhood" as Relay names it. SYNTHETIC, derived from the real mainnet fixtures where their
// structure allows: the SOL → 4663 and 4663 → SOL quotes are the real `sol-eth-20` / `eth-sol-0.5` with the 4663 side named and the
// order id recomputed (their amounts are Ethereum's); the sale receipts are the real Milady ones with CASHCAT's address written in; the
// 4663 Seaport order is signed by a test key over Seaport 1.6's domain on 4663 (no real 4663 fill is captured offline).
// =====================================================================================================================================

/// CASHCAT on Robinhood Chain (docs/v4/nft.md §1, Nosh's `CollectionRegistry`: a real ERC-721 on 4663).
const CASHCAT: [u8; 20] = [0xe3, 0xb3, 0x4c, 0x4b, 0xb0, 0xf1, 0x2c, 0x82, 0x14, 0x37, 0x45, 0xee, 0xe6, 0xa6, 0xcf, 0x4e, 0x31, 0x54, 0xb1, 0xfa];
const RH: u64 = 4663;

/// The sale receipts for `coll`: the real Milady ones, with the collection's address written as `coll` on another collection (synthetic).
fn sales_of(coll: [u8; 20]) -> String {
    if coll == MILADY { SALES.to_string() } else { SALES.replace(&hex::encode(MILADY), &hex::encode(coll)) }
}

/// Every `"ethereum"` refund of an order named `"robinhood"`, and the order id recomputed (`relayconv::order_id`).
fn rename_refunds_and_rehash(v: &mut Value) -> [u8; 32] {
    for r in v["protocol"]["v2"]["orderData"]["inputs"][0]["refunds"].as_array_mut().unwrap() {
        if r["chainId"] == "ethereum" {
            r["chainId"] = "robinhood".into();
        }
    }
    let id = etch_autobuyer::relayconv::order_id(&v["protocol"]["v2"]["orderData"]).unwrap();
    v["protocol"]["v2"]["orderId"] = format!("0x{}", hex::encode(id)).into();
    id
}

/// Synthetic: Relay's SOL → ETH quote with its output on Robinhood Chain (`"robinhood"`, 4663) and its order id recomputed in the
/// deposit instruction (`deposit_native`: discriminator ‖ amount ‖ id) and `protocol.v2.orderId`. As `relay_quote`, for our Relay key.
fn sol_to_rh(q: &str) -> Vec<u8> {
    let mut v: Value = serde_json::from_slice(&relay_quote(q)).unwrap();
    v["protocol"]["v2"]["orderData"]["output"]["chainId"] = "robinhood".into();
    let id = rename_refunds_and_rehash(&mut v);
    let ix = &mut v["steps"][0]["items"][0]["data"]["instructions"][0];
    let data = ix["data"].as_str().unwrap().to_string();
    ix["data"] = format!("{}{}", &data[..32], hex::encode(id)).into();
    v["details"]["currencyOut"]["currency"]["chainId"] = RH.into();
    serde_json::to_vec(&v).unwrap()
}

/// Synthetic: Relay's ETH → SOL quote with its origin Robinhood Chain: the deposit on 4663 (the same depository there), the order's
/// payment and origin refund on `"robinhood"`, its id recomputed in `depositNative(address, bytes32)` and `protocol.v2.orderId`.
fn rh_to_sol(q: &str) -> Vec<u8> {
    let mut v: Value = serde_json::from_str(q).unwrap();
    v["steps"][0]["items"][0]["data"]["chainId"] = RH.into();
    v["protocol"]["v2"]["orderData"]["inputs"][0]["payment"]["chainId"] = "robinhood".into();
    v["protocol"]["v2"]["paymentDetails"]["chainId"] = "robinhood".into();
    let id = rename_refunds_and_rehash(&mut v);
    let d = &mut v["steps"][0]["items"][0]["data"];
    let data = d["data"].as_str().unwrap().to_string();
    d["data"] = format!("{}{}", &data[..2 + 8 + 64], hex::encode(id)).into();
    serde_json::to_vec(&v).unwrap()
}

/// A Seaport 1.6 basic order selling `coll` #9180 for `ask` wei, signed by a test key over Seaport 1.6's EIP-712 domain on `chain`
/// (`fulfillBasicOrder` calldata, hex).
fn signed_order(chain: u64, coll: [u8; 20], ask: u128) -> String {
    use k256::ecdsa::{signature::hazmat::PrehashSigner, SigningKey};
    let sk = SigningKey::from_bytes(&[0x2a; 32].into()).unwrap();
    let p = sk.verifying_key().to_encoded_point(false);
    let me: [u8; 20] = keccak(&p.as_bytes()[1..])[12..].try_into().unwrap();
    let mut o = sp::BasicOrder { consideration_token: [0; 20], consideration_identifier: [0; 32], consideration_amount: ask, offerer: me,
        zone: [0; 20], offer_token: coll, offer_identifier: sp::word_u128(9_180), offer_amount: 1, basic_order_type: 0,
        start_time: sp::word_u128((NOW - 3_600) as u128), end_time: sp::word_u128((NOW + 30 * 86_400) as u128), zone_hash: [0; 32],
        salt: [7; 32], offerer_conduit_key: [0; 32], fulfiller_conduit_key: [0; 32], total_original_additional_recipients: 0,
        additional_recipients: vec![], signature: vec![] };
    let digest = keccak(&[&[0x19, 0x01][..], &sp::domain_separator(chain, &sp::SEAPORT_1_6), &o.components([0; 32]).hash()].concat());
    let (sig, rid): (k256::ecdsa::Signature, k256::ecdsa::RecoveryId) = sk.sign_prehash(&digest).unwrap();
    o.signature = [&sig.to_bytes()[..], &[27 + rid.to_byte()]].concat();
    format!("0x{}", hex::encode(sp::encode(&o)))
}

fn evm_chain(id: u64) -> etch_autobuyer::chains::EvmChain {
    etch_autobuyer::chains::EvmChain { chain_id: id, rpc_urls: vec!["https://a".into(), "https://b".into()], bucket: 8,
        seaport: "0x0000000000000068F116a894984e2DB1123eB395".into(), seaport_codehash: format!("0x{}", "ab".repeat(32)),
        max_fee_wei: 200_000_000_000, priority_fee_wei: 1_000_000_000, gas_cap: 400_000, fee_recipients: vec![], max_fee_bps: 1_500,
        min_order_life_s: 300, signed_zones: None, zone_registries: None }
}

fn rh_col(coll: [u8; 20], chain: Option<u64>) -> EthCollection {
    EthCollection { collection: format!("0x{}", hex::encode(coll)), wrapper: "books".into(), module: "books".into(), max_purchase_wei: 2 * E,
        max_premium_bps: 500, chain_id: chain, token_bound: None, nft_value_bps: None }
}

/// The 4663 side to arrival: the advance, the SOL → 4663 Relay quote judged and deposited, 0.87 ETH arrived at our address on 4663.
fn rh_to_arrival(w: &mut World, st: &mut EthState) {
    books_to_funded(w, st);
    let id = st.buys[0].id;
    assert_eq!(id, ef::side_buy_base(RH), "the 4663 side's buys count from its own base");
    // the mainnet quote (paying out on "ethereum") is refused on this side: S9, another chain; nothing is consumed but the quote
    ef::put_quote(st, id, relay_quote(SOL_ETH_20), w.now).unwrap();
    let l = tick(w, st);
    assert!(l.iter().any(|x| x.contains("rule S8") || x.contains("rule S9")), "{l:?}");
    assert!(matches!(phase(st), BPhase::Funded { .. }), "{:?}", phase(st));
    ef::put_quote(st, id, sol_to_rh(SOL_ETH_20), w.now).unwrap();
    let l = tick(w, st);
    assert!(matches!(phase(st), BPhase::Depositing { .. }), "{l:?}");
    assert_eq!(w.land().len(), 1, "the funding of the Relay key lands");
    tick(w, st);
    assert_eq!(w.land().len(), 1, "the deposit lands");
    tick(w, st);
    let BPhase::Bridging { value, min_out, .. } = phase(st) else { panic!("{:?}", phase(st)) };
    assert_eq!((value, min_out), (20_000_000_000, 859_035_589_742_793_043));
    assert_eq!(w.sys.borrow().deposits.len(), 1, "Relay's depository took the deposit");
    w.eth_balance += 870_000_000_000_000_000;
    tick(w, st);
    let BPhase::Arrived { wei, .. } = phase(st) else { panic!("{:?}", phase(st)) };
    assert_eq!(wei, 870_000_000_000_000_000);
}

fn rh_evidence(w: &World, st: &mut EthState) {
    ef::put_evidence(st, &w.cfg(), CASHCAT, vec![format!("1:{}", 92 * E / 100), format!("2:{}", 93 * E / 100), format!("3:{}", 95 * E / 100)],
        sales(), w.now).unwrap();
}

#[test]
fn rh_the_install_takes_collections_on_two_evm_chains_and_refuses_a_collection_whose_chain_is_not_listed() {
    let cfg = EthNftConfig { chain_id: 1, relay_max_fee_bps: 300, deposits: false, floor: None, gas_pot_target_wei: None, logs_max_blocks: None,
        dormancy: None, collections: vec![rh_col(MILADY, None), rh_col(CASHCAT, Some(RH))] };
    let chains = vec![evm_chain(1), evm_chain(RH)];
    assert_eq!(etch_autobuyer::chains::validate(&chains, 0), Ok(()), "on the Solana home every EVM chain may be listed, 4663 included");
    assert_eq!(en::validate(&cfg, &chains, 0), Ok(()));
    assert_eq!(en::validate_home(&cfg, true), Ok(()));
    assert!(en::validate_home(&cfg, false).unwrap_err().contains("several chains"), "the EVM home has no Robinhood Chain leg");
    assert_eq!(cfg.chains(), vec![1, RH]);
    let (eth, rh) = (cfg.on(1), cfg.on(RH));
    assert_eq!((eth.chain_id, eth.collections.len(), eth.collections[0].collection.clone()), (1, 1, format!("0x{}", hex::encode(MILADY))));
    assert_eq!((rh.chain_id, rh.collections.len(), rh.collections[0].collection.clone()), (RH, 1, format!("0x{}", hex::encode(CASHCAT))));
    assert_eq!(cfg.chain_of_addr(&CASHCAT), Some(RH));
    assert_eq!(cfg.chain_of_addr(&MILADY), Some(1));
    // a mainnet-only argument is its own Ethereum side, unchanged
    let one = EthNftConfig { collections: vec![rh_col(MILADY, None)], ..cfg.clone() };
    assert_eq!((one.chains(), one.on(1)), (vec![1], one.clone()));
    // refused: 4663 not in evm_chains; a chain Relay's legs do not name; the dormancy across chains; one address twice
    assert!(en::validate(&cfg, &[evm_chain(1)], 0).unwrap_err().contains("not one of evm_chains"));
    let base = EthNftConfig { collections: vec![rh_col(MILADY, None), rh_col(CASHCAT, Some(8453))], ..cfg.clone() };
    assert!(en::validate(&base, &[evm_chain(1), evm_chain(8453)], 0).unwrap_err().contains("Relay legs do not name"));
    let dorm = EthNftConfig { dormancy: Some(Dormancy::launch(None)), ..cfg.clone() };
    assert!(en::validate(&dorm, &chains, 0).unwrap_err().contains("several chains"));
    let twice = EthNftConfig { collections: vec![rh_col(CASHCAT, None), rh_col(CASHCAT, Some(RH))], ..cfg.clone() };
    assert!(en::validate(&twice, &chains, 0).unwrap_err().contains("listed twice"));
    // an install argument from before the field decodes, every collection on the config's chain
    #[derive(candid::CandidType)]
    struct OldCol { collection: String, wrapper: String, module: String, max_purchase_wei: u128, max_premium_bps: u16 }
    let old = OldCol { collection: format!("0x{}", hex::encode(MILADY)), wrapper: "books".into(), module: "books".into(), max_purchase_wei: 2 * E,
        max_premium_bps: 500 };
    let back: EthCollection = candid::decode_one(&candid::encode_one(&old).unwrap()).unwrap();
    assert_eq!(back, rh_col(MILADY, None));
    // the Relay legs: the chain names Relay's orders use, and the way back from each
    assert_eq!((etch_autobuyer::relayeth::relay_name(1), etch_autobuyer::relayeth::relay_name(RH)), (Some("ethereum"), Some("robinhood")));
    assert_eq!(etch_autobuyer::relayeth::to_solana(RH).map(|l| (l.origin, l.origin_chain, l.destination)), Some(("robinhood", RH, "solana")));
    assert_eq!(etch_autobuyer::relayeth::to_solana(1), Some(etch_autobuyer::relayeth::ETHEREUM_TO_SOLANA));
}

#[test]
fn rh_two_sides_share_the_line_each_takes_only_its_own_outcomes() {
    let all = EthNftConfig { chain_id: 1, relay_max_fee_bps: 300, deposits: false, floor: None, gas_pot_target_wei: None, logs_max_blocks: None,
        dormancy: None, collections: vec![rh_col(MILADY, None), rh_col(CASHCAT, Some(RH))] };
    let (eth, rh) = (all.on(1), all.on(RH));
    let mut a = EthState::default();
    let mut b = ef::new_side(RH);
    assert_eq!(b.next_buy, 4663u64 << 40);
    let buy = |id, c: [u8; 20]| ef::EthBuy { id, collection: c, module: [0; 20], wrapper: [0; 20], advanced: 0, attempts: 0, phase: BPhase::Advancing,
        orders: vec![], deposits: vec![], ask: None, last: None, note: String::new(), sweep: None, arrived_at: None, back_at: None, ended_at: None, late: None };
    a.buys.push(buy(0, MILADY));
    b.buys.push(buy(b.next_buy, CASHCAT));
    let (sa, sb) = (ef::Step::Advance { buy: 0 }, ef::Step::Advance { buy: b.next_buy });
    assert!(ef::owns_step(&a, &eth, &sa) && !ef::owns_step(&a, &eth, &sb));
    assert!(ef::owns_step(&b, &rh, &sb) && !ef::owns_step(&b, &rh, &sa));
    let (pa, pb) = (ef::Step::Post { collection: MILADY }, ef::Step::Post { collection: CASHCAT });
    assert!(ef::owns_step(&a, &eth, &pa) && !ef::owns_step(&a, &eth, &pb) && ef::owns_step(&b, &rh, &pb));
    let wr = rh.cols().unwrap()[0].wrapper;
    let rec = ef::Step::Record { wrapper: wr, ticket: 0 };
    assert!(ef::owns_step(&b, &rh, &rec) && !ef::owns_step(&a, &eth, &rec));
}

#[test]
fn rh_a_floor_post_on_4663_is_a_book_update_from_cashcats_listings_and_sales() {
    let mut w = World::books_on(RH, CASHCAT);
    let mut st = ef::new_side(RH);
    rh_to_arrival(&mut w, &mut st);
    let before = w.bmodule().accepted;
    rh_evidence(&w, &mut st);
    let l = tick(&mut w, &mut st);
    let r = st.reading(&CASHCAT).unwrap().clone();
    assert!(r.post.price > 0, "a reading of CASHCAT from its listings and its sales on 4663");
    assert!(l.iter().any(|x| x.contains(&format!("eth floor 0x{}: post_floor(", hex::encode(CASHCAT))) && x.contains("floor posted in the books")), "{l:?}");
    assert!(w.bmodule().accepted > before, "the books' module moved");
    assert_eq!(w.land().len(), 0, "no transaction for a post");
}

#[test]
fn rh_a_buy_end_to_end_on_4663_relay_sol_to_robinhood_seaport_1_6_on_4663_mint_item_settle() {
    let mut w = World::books_on(RH, CASHCAT);
    let mut st = ef::new_side(RH);
    rh_to_arrival(&mut w, &mut st);
    let id = st.buys[0].id;
    rh_evidence(&w, &mut st);
    ef::put_listing(&mut st, id, signed_order(RH, CASHCAT, 45 * E / 100), w.now).unwrap();
    tick(&mut w, &mut st);
    let BPhase::Buying { total, .. } = st.buys[0].phase.clone() else { panic!("{:?}", st.buys[0].phase.clone()) };
    assert_eq!(total, 45 * E / 100);
    tick(&mut w, &mut st);
    let (tx, _) = w.eth_signed[0].clone();
    assert_eq!((tx.chain_id, tx.to, tx.value), (RH, sp::SEAPORT_1_6, 45 * E / 100), "the fill is signed for 4663's Seaport 1.6");
    w.mine_eth(0, true, ETH_PIN - 10);
    tick(&mut w, &mut st);
    assert!(matches!(st.buys[0].phase.clone(), BPhase::Bought { .. }), "{:?}", st.buys[0].phase.clone());
    w.finalized = ETH_PIN;
    let l = tick(&mut w, &mut st);
    assert!(l.iter().any(|x| x.contains("minted to the module in the books")), "{l:?}");
    assert_eq!(w.ledger().items.get(&0).map(|x| x.key), Some(sp::word_u128(9_180)));
    tick(&mut w, &mut st);
    let l = tick(&mut w, &mut st);
    assert!(matches!(st.buys[0].phase.clone(), BPhase::Settled { .. }), "{:?} {l:?}", st.buys[0].phase.clone());
    let (m, ld) = (w.bmodule(), w.ledger());
    assert_eq!((m.transit_units, m.items_arrived, ld.module_units, ld.held), (0, 1, 0, 1), "settled: the unit is item-backed");
    // custody: the item is booked under its own chain's key (`evm:4663:0x…`)
    assert!(w.book.contains(&format!("{}:{}", custodycap::evm_key(RH, &CASHCAT), hex::encode(sp::word_u128(9_180)))), "{:?}", w.book);
    assert!(!w.book.contains(&format!("{}:{}", custodycap::evm_key(1, &CASHCAT), hex::encode(sp::word_u128(9_180)))));
}

#[test]
fn rh_an_order_signed_for_another_chains_seaport_is_refused_on_4663() {
    // pure: the domain carries the chain id; Ethereum's real Milady fill is not its offerer's on 4663
    assert_ne!(sp::domain_separator(RH, &sp::SEAPORT_1_6), sp::domain_separator(1, &sp::SEAPORT_1_6));
    let real = sp::decode(&hex::decode(fill_hex().trim_start_matches("0x")).unwrap()).unwrap();
    assert!(sp::signed_by_offerer(&real, RH, &sp::SEAPORT_1_6, [0; 32]).unwrap_err().contains("not the offerer"));
    // the flow: a CASHCAT order signed over Ethereum's Seaport domain is refused on the 4663 side; nothing is signed; the 4663 one goes
    let mut w = World::books_on(RH, CASHCAT);
    let mut st = ef::new_side(RH);
    rh_to_arrival(&mut w, &mut st);
    let id = st.buys[0].id;
    rh_evidence(&w, &mut st);
    ef::put_listing(&mut st, id, signed_order(1, CASHCAT, 45 * E / 100), w.now).unwrap();
    let l = tick(&mut w, &mut st);
    assert!(w.eth_signed.is_empty(), "nothing signed: {l:?}");
    assert!(l.iter().any(|x| x.contains("not the offerer")) || st.buys[0].note.contains("not the offerer"), "{l:?} {}", st.buys[0].note);
    assert!(matches!(st.buys[0].phase, BPhase::Arrived { .. }), "{:?}", st.buys[0].phase);
    // another collection's order is refused too (the 4663 side buys CASHCAT only)
    ef::put_listing(&mut st, id, signed_order(RH, MILADY, 45 * E / 100), w.now).unwrap();
    let l = tick(&mut w, &mut st);
    assert!(w.eth_signed.is_empty(), "nothing signed: {l:?}");
    ef::put_listing(&mut st, id, signed_order(RH, CASHCAT, 45 * E / 100), w.now).unwrap();
    let l = tick(&mut w, &mut st);
    assert!(matches!(st.buys[0].phase, BPhase::Buying { .. }), "{:?} {l:?}", st.buys[0].phase);
}

#[test]
fn rh_the_way_back_from_4663_is_relays_robinhood_to_solana_leg_deposited_on_4663() {
    let mut w = World::books_on(RH, CASHCAT);
    let mut st = ef::new_side(RH);
    rh_to_arrival(&mut w, &mut st);
    let id = st.buys[0].id;
    w.now += ef::ARRIVED_BACK_SECS + 1;
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st), BPhase::BackFunded { .. }), "{:?}", phase(&st));
    let ask = st.buys[0].ask.clone().unwrap();
    assert_eq!((ask.origin_chain_id, ask.destination_chain_id), (RH, relaysol::RELAY_SOLANA), "the runner is asked for 4663 → Solana");
    st.buys[0].ask.as_mut().unwrap().amount_wei = 500_000_000_000_000_000;
    let q: Value = serde_json::from_str(ETH_SOL).unwrap();
    w.now = q["protocol"]["v2"]["orderData"]["output"]["deadline"].as_u64().unwrap() - 7 * 86_400 + 5;
    st.buys[0].phase = BPhase::BackFunded { wei: 870_000_000_000_000_000, since: w.now };
    // Ethereum's leg (a deposit on chain 1) is refused on this side: rule 2
    ef::put_quote(&mut st, id, ETH_SOL.as_bytes().to_vec(), w.now).unwrap();
    let l = tick(&mut w, &mut st);
    assert!(w.eth_signed.is_empty(), "{l:?}");
    assert!(st.buys[0].note.contains("rule 2") || l.iter().any(|x| x.contains("rule 2")), "{l:?} {}", st.buys[0].note);
    // 4663's: judged, the deposit signed on 4663 to Relay's depository there
    ef::put_quote(&mut st, id, rh_to_sol(ETH_SOL), w.now).unwrap();
    let l = tick(&mut w, &mut st);
    assert!(matches!(phase(&st), BPhase::BackSending { .. }), "{:?} {l:?}", phase(&st));
    let (tx, _) = w.eth_signed.last().unwrap().clone();
    assert_eq!((tx.chain_id, tx.to, tx.value), (RH, etch_autobuyer::relayconv::RELAY_DEPOSITORY_4663, 500_000_000_000_000_000));
}

#[test]
fn rh_a_delivery_on_4663_is_an_erc721_transfer_signed_for_4663_and_recorded_in_the_books() {
    let mut w = World::books_on(RH, CASHCAT);
    let mut st = ef::new_side(RH);
    rh_to_arrival(&mut w, &mut st);
    let id = st.buys[0].id;
    rh_evidence(&w, &mut st);
    ef::put_listing(&mut st, id, signed_order(RH, CASHCAT, 45 * E / 100), w.now).unwrap();
    tick(&mut w, &mut st);
    tick(&mut w, &mut st);
    w.mine_eth(0, true, ETH_PIN - 10);
    tick(&mut w, &mut st);
    w.finalized = ETH_PIN;
    for _ in 0..3 {
        tick(&mut w, &mut st);
    }
    assert!(matches!(st.buys[0].phase.clone(), BPhase::Settled { .. }), "{:?}", st.buys[0].phase.clone());
    // a holder redeems one CASHCAT unit to an EVM address; the ticket resolves to the item
    let dest = [0x77; 20];
    let t = etch_autobuyer::desk::state::with_mut(|s| s.books.redeem(1, [0x55; 32], etch_autobuyer::books::ledger::Destination::Eth(dest), w.now as i64))
        .unwrap();
    etch_autobuyer::desk::state::with_mut(|s| s.books.resolve(1, t, Some([9; 32]), w.now as i64 + 3_600)).unwrap();
    w.now += 3_700;
    let mut signed = None;
    for _ in 0..4 {
        let l = tick(&mut w, &mut st);
        if let Some(i) = w.eth_signed.iter().position(|(tx, _)| tx.to == CASHCAT) {
            signed = Some(i);
            break;
        }
        println!("{l:?}");
    }
    let i = signed.expect("the delivery is signed");
    let (tx, _) = w.eth_signed[i].clone();
    assert_eq!((tx.chain_id, tx.to, tx.value), (RH, CASHCAT, 0), "an ERC-721 transfer on 4663");
    assert_eq!(tx.data, en::transfer_from_call(&US, &dest, sp::word_u128(9_180)));
    w.mine_eth(i, true, ETH_PIN + 1);
    w.finalized = ETH_PIN + 200;
    let mut done = false;
    for _ in 0..6 {
        tick(&mut w, &mut st);
        if w.ledger().ticket(t).unwrap().stage == etch_autobuyer::books::ledger::Stage::Delivered {
            done = true;
            break;
        }
    }
    assert!(done, "recorded delivered in the books: {:?}", st.deliveries);
    assert_eq!(w.owners.get(&sp::word_u128(9_180)), Some(&dest));
}

// ---- StonkBrokers (the operator, 2026-10-12: included, token-bound) ---------------------------------------------------------------------

/// StonkBrokers on Robinhood Chain (docs/v4/nft.md §1): each NFT is an ERC-6551 account holding tokenized stocks.
const STONK: [u8; 20] = [0x53, 0x9c, 0xdd, 0x04, 0x2c, 0x2f, 0x3d, 0x93, 0xeb, 0xc5, 0xbe, 0x7d, 0xff, 0xf0, 0xc3, 0xbc, 0x5c, 0x4f, 0xab, 0xf0];

/// A high-floor collection is bought within its OWN purchase bound (StonkBrokers ≈ 11–13 ETH, its bound 15 ETH) while another collection
/// keeps its own (2 ETH); a StonkBrokers-priced item over its bound, or a bound over the 25 ETH ceiling, is refused. Token-bound: priced
/// as the NFT (its listings and the module's floor), never by the account's contents; the custody caps still bound the total.
#[test]
fn rh_stonkbrokers_a_token_bound_high_floor_item_within_its_own_cap_and_refused_above_it() {
    let stonk = EthCollection { max_purchase_wei: 15 * E, token_bound: Some(true), nft_value_bps: Some(2_000), ..rh_col(STONK, Some(RH)) };
    let cfg = EthNftConfig { chain_id: 1, relay_max_fee_bps: 300, deposits: false, floor: None, gas_pot_target_wei: None, logs_max_blocks: None,
        dormancy: None, collections: vec![rh_col(MILADY, None), rh_col(CASHCAT, Some(RH)), stonk.clone()] };
    let chains = vec![evm_chain(1), evm_chain(RH)];
    assert_eq!(en::validate(&cfg, &chains, 0), Ok(()));
    assert!(stonk.is_token_bound() && !rh_col(CASHCAT, Some(RH)).is_token_bound());
    assert_eq!(cfg.on(RH).collections.len(), 2, "CASHCAT and StonkBrokers on the 4663 side");
    // the bound is the collection's own, within [0.05 ETH, 25 ETH]
    let over = EthNftConfig { collections: vec![EthCollection { max_purchase_wei: 26 * E, ..stonk.clone() }], ..cfg.clone() };
    assert!(en::validate(&over, &chains, 0).unwrap_err().contains("max_purchase_wei"));
    // the buy bound: a 12 ETH reading and accepted floor, 5% premium, 13.2 ETH arrived
    let now = NOW;
    let own = Some((12 * E, now - 60));
    let cap = en::buy_cap(own, 12 * E, now, 500, 132 * E / 10, stonk.max_purchase_wei).unwrap();
    assert_eq!(cap, 126 * E / 10, "the floor plus the premium, under its own 15 ETH bound");
    // a floor above the collection's own bound: the bound decides (never the account's contents)
    assert_eq!(en::buy_cap(Some((20 * E, now - 60)), 20 * E, now, 500, 22 * E, stonk.max_purchase_wei).unwrap(), 15 * E);
    // the same item under CASHCAT's 2 ETH bound
    assert_eq!(en::buy_cap(own, 12 * E, now, 500, 132 * E / 10, rh_col(CASHCAT, Some(RH)).max_purchase_wei).unwrap(), 2 * E);
    // the 4663 side's Seaport judge: a signed 12.5 ETH StonkBrokers order is planned within the cap, a 13 ETH one refused
    let mut w = World::books_on(RH, STONK);
    block_on(async {
        assert_eq!(w.eth_plan_buy(&signed_order(RH, STONK, 125 * E / 10), STONK, cap).await.unwrap().total, 125 * E / 10);
        assert!(w.eth_plan_buy(&signed_order(RH, STONK, 13 * E), STONK, cap).await.unwrap_err().contains("over cap"));
        // the delivery of a token-bound NFT is the ERC-721 transfer alone, on the collection (the account goes with it)
        let p = w.eth_plan_transfer(STONK, sp::word_u128(9_180), [0x77; 20]).await.unwrap();
        assert_eq!((p.to, p.value, p.data.clone()), (STONK, 0, en::transfer_from_call(&US, &[0x77; 20], sp::word_u128(9_180))));
    });
}
