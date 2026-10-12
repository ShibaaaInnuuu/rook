//! Permissionless offer hand-ins (docs/v6/offer-handins.md) against a fake world: an OfferBoard that checks what the real one
//! checks (open ids, the custodian's claim and slash, the slash delay on both clocks), a funded buy queue, a wrapper, a
//! Bitcoin that confirms and reorganises, ord and explorers, and bots that hand in offers good, bad, dear and dead.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use etch_autobuyer::btcnet::BtcSeen;
use etch_autobuyer::btcsend::parse_signed;
use etch_autobuyer::custody::{Contents, Location, Outpoint, Output, Utxo};
use etch_autobuyer::deposits::{self, TxView};
use etch_autobuyer::evmtx;
use etch_autobuyer::floortrigger::FloorState;
use etch_autobuyer::frogbuy::{self, FPhase};
use etch_autobuyer::frogflow::{self, FStep, FrogIo, FrogState, Offer};
use etch_autobuyer::handin::{self, HandinState, Verdict};
use etch_autobuyer::itemsink::{Clock, SinkKind, SinkRec};
use etch_autobuyer::looper::EvmView;
use etch_autobuyer::psbt::{Psbt, Tx, TxIn, TxOut};
use etch_autobuyer::{buyqueue, curvebuy, custodycap, outbound, outflow};

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
const T0: u64 = 1_800_000_000;
const B0: u64 = 26_000_000;
const ME: [u8; 20] = [0xcc; 20];
const W: [u8; 20] = [0xaa; 20];
const REG: [u8; 20] = [0xab; 20];
const QUEUE: [u8; 20] = [0x51; 20];
const BOARD: [u8; 20] = [0x0b; 20];
const BOT: [u8; 20] = [0xb1; 20];
const BOT2: [u8; 20] = [0xb2; 20];
const RATE: u64 = 3_190_000; // sats per ETH
const PRICE: u64 = 900_000; // sats to the seller
const BOND: u128 = 10_000_000_000_000_000;
const REWARD: u128 = 10_000_000_000_000_000;
const SLASH_DELAY: u64 = 3_600;
const EXPIRY: u64 = 86_400;

fn p2tr(tag: u8) -> Vec<u8> {
    let mut s = vec![0x51, 0x20];
    s.extend([tag; 32]);
    s
}
fn float() -> Vec<u8> {
    p2tr(0xf1)
}
fn custody() -> Vec<u8> {
    p2tr(0xc2)
}
fn insc(n: u8) -> String {
    format!("{}i0", hex::encode([n; 32]))
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
fn rlp_items(b: &[u8]) -> Vec<Vec<u8>> {
    fn item(b: &[u8], i: &mut usize) -> Vec<u8> {
        let p = b[*i];
        let (start, len) = match p {
            0x00..=0x7f => (*i, 1),
            0x80..=0xb7 => (*i + 1, (p - 0x80) as usize),
            0xb8..=0xbf => { let n = (p - 0xb7) as usize; (*i + 1 + n, b[*i + 1..*i + 1 + n].iter().fold(0, |a, &x| a * 256 + x as usize)) }
            0xc0..=0xf7 => (*i + 1, (p - 0xc0) as usize),
            _ => { let n = (p - 0xf7) as usize; (*i + 1 + n, b[*i + 1..*i + 1 + n].iter().fold(0, |a, &x| a * 256 + x as usize)) }
        };
        *i = start + len;
        b[start..start + len].to_vec()
    }
    let mut i = 1;
    let list = item(b, &mut i);
    let mut j = 0;
    let mut out = Vec::new();
    while j < list.len() {
        out.push(item(&list, &mut j));
    }
    out
}
fn uint(b: &[u8]) -> u128 {
    b.iter().fold(0u128, |a, &x| a * 256 + x as u128)
}
fn arg(data: &[u8], n: usize) -> u128 {
    u128::from_be_bytes(data[4 + 32 * n + 16..4 + 32 * n + 32].try_into().unwrap())
}
fn arg_word(data: &[u8], n: usize) -> [u8; 32] {
    data[4 + 32 * n..4 + 32 * n + 32].try_into().unwrap()
}
fn display(internal: [u8; 32]) -> [u8; 32] {
    let mut d = internal;
    d.reverse();
    d
}

// ---- the world ------------------------------------------------------------------------------------------------------------------

/// `OfferBoard.Submission`, as the contract keeps it.
#[derive(Clone, Debug)]
struct Sub {
    submitter: [u8; 20],
    at: u64,
    at_block: u64,
    status: u8,
    ask: u64,
    hash: [u8; 32],
    inscription: String,
}

struct BtcTx {
    height: Option<u64>,
    hash: Option<String>,
}

struct World {
    now: u64,
    clock: Clock,
    root: [u8; 32],
    minted: HashSet<[u8; 32]>,
    held: HashSet<[u8; 32]>,
    balances: HashMap<[u8; 20], u128>,
    paid: u128,
    queued: u128,
    cap: u128,
    recorded: u64,
    // the board
    subs: Vec<Sub>,
    owed: HashMap<[u8; 20], u128>,
    pool: u128,
    board_wrapper: [u8; 20],
    // the EVM
    nonce: u64,
    mempool: Vec<Vec<u8>>,
    receipts: HashMap<[u8; 32], bool>,
    calls: Vec<([u8; 20], Vec<u8>, bool)>,
    // Bitcoin
    tip: u64,
    blocks: HashMap<u64, String>,
    txs: HashMap<[u8; 32], BtcTx>,
    loc: HashMap<String, Location>,
    contents: HashMap<Outpoint, Contents>,
    outputs: HashMap<Outpoint, (Output, bool)>,
    outspends: HashMap<Outpoint, ([u8; 32], bool)>,
    utxos: Vec<Utxo>,
    btc_sent: Vec<Vec<u8>>,
    offers: HashMap<u64, Offer>,
    saved: Option<FrogState>,
    saved_handins: Option<HandinState>,
    caps: custodycap::CapPolicy,
    book: custodycap::Book,
    limits: Option<outflow::Limits>,
    outflow: outflow::Outflow,
    asked: Vec<(String, outflow::Flow, u128)>,
    /// How many times the float key was asked to sign.
    float_signs: usize,
}

fn block_hash_for(h: u64) -> String {
    hex::encode(evmtx::keccak(&h.to_be_bytes()))
}

impl World {
    fn new() -> Self {
        let k = custodycap::btc_key(&W);
        let caps = custodycap::CapPolicy { collections: vec![custodycap::CollectionCap { collection: k, max_items: 1_000,
            max_value_wei: 1_000_000 * E, max_item_wei: 1_000 * E }], global_max_items: 1_000, global_max_value_wei: 1_000_000 * E };
        let mut w = World {
            now: T0, clock: Clock { time: T0, parent_block: B0 }, root: [0; 32], minted: HashSet::new(), held: HashSet::new(),
            balances: HashMap::new(), paid: 0,
            // a funded queue: 0.42 ETH, cap 0.40, buffer 0.02
            queued: 42 * E / 100, cap: 40 * E / 100, recorded: 0,
            subs: vec![], owed: HashMap::new(), pool: 1 * E, board_wrapper: W,
            nonce: 0, mempool: vec![], receipts: HashMap::new(), calls: vec![],
            tip: 800_000, blocks: HashMap::new(), txs: HashMap::new(), loc: HashMap::new(), contents: HashMap::new(),
            outputs: HashMap::new(), outspends: HashMap::new(), utxos: vec![], btc_sent: vec![], offers: HashMap::new(),
            saved: None, saved_handins: None, caps, book: custodycap::Book::default(), limits: None,
            outflow: outflow::Outflow::default(), asked: vec![], float_signs: 0,
        };
        w.blocks.insert(w.tip, block_hash_for(w.tip));
        w
    }
    fn advance(&mut self, secs: u64) {
        self.now += secs;
        self.clock.time += secs;
        self.clock.parent_block += secs / 12;
    }
    fn collection(&mut self, ids: &[String]) -> Vec<Vec<[u8; 32]>> {
        assert_eq!(ids.len(), 4);
        let pair = |a: [u8; 32], b: [u8; 32]| { let (x, y) = if a <= b { (a, b) } else { (b, a) }; evmtx::keccak(&[x, y].concat()) };
        let l: Vec<[u8; 32]> = ids.iter().map(|i| frogbuy::leaf_of(i)).collect();
        let (a, b) = (pair(l[0], l[1]), pair(l[2], l[3]));
        self.root = pair(a, b);
        vec![vec![l[1], b], vec![l[0], b], vec![l[3], a], vec![l[2], a]]
    }
    fn list(&mut self, id: &str, seller: Outpoint) {
        self.loc.insert(id.to_string(), Location { output: seller, offset: 0, value: 10_000 });
        self.contents.insert(seller, Contents { inscriptions: vec![id.to_string()], runes: vec![], value: 10_000 });
        self.outputs.insert(seller, (Output { script: p2tr(0xee), value: 10_000, confirmed: true }, false));
    }
    /// The marketplace's PSBT: padding, the frog, a payment input; out: padding, the frog to custody, the price, change.
    /// `salt` makes distinct bytes (and hashes) for the same listing.
    fn psbt_for(&self, salt: u8, seller: Outpoint, price: u64, op_return: bool) -> (Vec<u8>, Vec<usize>) {
        let mut outputs = vec![
            TxOut { value: 1_200, script: float() },
            TxOut { value: 10_000, script: custody() },
            TxOut { value: price, script: p2tr(0xe5) },
            TxOut { value: 10_000_000 - price - 3_000, script: float() },
        ];
        if op_return {
            outputs.push(TxOut { value: 0, script: vec![0x6a, 0x5d, 0x01, 0x00] });
        }
        let tx = Tx {
            version: 2,
            inputs: vec![
                TxIn { txid: [0xa1; 32], vout: salt as u32, sequence: 0xffff_fffd },
                TxIn { txid: [0xa2; 32], vout: salt as u32, sequence: 0xffff_fffd },
                TxIn { txid: seller.internal_txid(), vout: seller.vout, sequence: 0xffff_fffd },
                TxIn { txid: [0xa3; 32], vout: salt as u32, sequence: 0xffff_fffd },
            ],
            outputs,
            locktime: 0,
        };
        let utxos = vec![Some(TxOut { value: 600, script: float() }), Some(TxOut { value: 600, script: float() }),
            Some(TxOut { value: 10_000, script: p2tr(0xee) }), Some(TxOut { value: 10_000_000, script: float() })];
        let p = Psbt { tx, witness_utxos: utxos };
        // the seller's key-path signature under SIGHASH_SINGLE|ANYONECANPAY (0x83), the listing shape the judge completes
        let mut seller_sig = vec![7u8; 65];
        seller_sig[64] = 0x83;
        let finals = vec![None, None, Some(vec![seller_sig]), None];
        (frogbuy::psbt_bytes(&p, &finals), vec![0, 1, 3])
    }
    /// A bot's two steps: `submit` on the board (a bond, the hash, the ask), and the bytes to the canister (`submit_offer`).
    fn hand_in(&mut self, hs: &mut HandinState, who: [u8; 20], bytes: &[u8], to_sign: &[usize], ask: u64, id: &str, proof: Vec<[u8; 32]>,
        deliver: bool) -> u64 {
        let sub_id = self.submit(who, handin::offer_hash(bytes), ask, id);
        if deliver {
            hs.intake(self.now, bytes.to_vec(), to_sign.iter().map(|i| *i as u32).collect(), proof).unwrap();
        }
        sub_id
    }
    fn submit(&mut self, who: [u8; 20], hash: [u8; 32], ask: u64, id: &str) -> u64 {
        assert!(!self.subs.iter().any(|s| s.status == 1 && s.hash == hash), "Duplicate");
        self.subs.push(Sub { submitter: who, at: self.clock.time, at_block: self.clock.parent_block, status: 1, ask, hash,
            inscription: id.to_string() });
        (self.subs.len() - 1) as u64
    }
    fn slashable(&self, s: &Sub) -> bool {
        s.status == 1 && self.clock.time >= s.at + SLASH_DELAY && self.clock.parent_block >= s.at_block + SLASH_DELAY / 12
    }
    fn expired(&self, s: &Sub) -> bool {
        self.clock.time >= s.at + EXPIRY && self.clock.parent_block >= s.at_block + EXPIRY / 12
    }
    /// The submitter takes the bond back after expiry.
    fn refund(&mut self, id: u64) {
        let s = self.subs[id as usize].clone();
        assert!(s.status == 1 && self.expired(&s), "TooEarly");
        self.subs[id as usize].status = 4;
        *self.owed.entry(s.submitter).or_default() += BOND;
    }
    fn open_ids(&self) -> Vec<u64> {
        self.subs.iter().enumerate().filter(|(_, s)| s.status == 1).map(|(i, _)| i as u64).collect()
    }
    fn encode_sub(s: &Sub) -> Vec<u8> {
        let mut b = word(0x20);
        b.extend(aword(&s.submitter));
        b.extend(word(s.at as u128));
        b.extend(word(s.at_block as u128));
        b.extend(word(s.status as u128));
        b.extend(word(BOND));
        b.extend(word(s.ask as u128));
        b.extend_from_slice(&s.hash);
        b.extend(word(0x100));
        b.extend(word(s.inscription.len() as u128));
        let mut str_bytes = s.inscription.as_bytes().to_vec();
        str_bytes.resize(str_bytes.len().div_ceil(32) * 32, 0);
        b.extend(str_bytes);
        b
    }
    fn mine_btc(&mut self, n: u64) {
        for k in 0..n {
            self.tip += 1;
            let h = self.tip;
            self.blocks.insert(h, block_hash_for(h));
            if k > 0 {
                continue;
            }
            let pending: Vec<[u8; 32]> = self.txs.iter().filter(|(_, t)| t.height.is_none()).map(|(id, _)| *id).collect();
            for id in pending {
                let hash = self.blocks[&h].clone();
                let t = self.txs.get_mut(&id).unwrap();
                t.height = Some(h);
                t.hash = Some(hash);
                self.settle_btc(id);
            }
        }
    }
    /// A reorganisation: the purchase's block is replaced and the transaction is back in the mempool.
    fn reorg_out(&mut self, txid: [u8; 32]) {
        let t = self.txs.get_mut(&txid).unwrap();
        let h = t.height.take().unwrap();
        t.hash = None;
        self.blocks.insert(h, format!("ff{}", &block_hash_for(h)[2..]));
    }
    fn settle_btc(&mut self, txid: [u8; 32]) {
        let Some(raw) = self.btc_sent.iter().find(|r| display(parse_signed(r).unwrap().txid) == txid).cloned() else { return };
        let tx = parse_signed(&raw).unwrap();
        if tx.outputs.get(1) != Some(&(10_000, custody())) {
            return;
        }
        for i in &tx.inputs {
            let spent = Outpoint { txid: display(i.txid), vout: i.vout };
            if let Some((id, _)) = self.loc.iter().find(|(_, l)| l.output == spent).map(|(a, b)| (a.clone(), b.clone())) {
                let at = Outpoint { txid, vout: 1 };
                self.loc.insert(id.clone(), Location { output: at, offset: 0, value: 10_000 });
                self.contents.insert(at, Contents { inscriptions: vec![id.clone()], runes: vec![], value: 10_000 });
                self.outputs.insert(at, (Output { script: custody(), value: 10_000, confirmed: true }, false));
                self.utxos.push(Utxo { outpoint: at, value: 10_000, confirmed: true });
                self.outspends.insert(spent, (txid, true));
                if let Some(o) = self.outputs.get_mut(&spent) {
                    o.1 = true;
                }
            }
        }
    }
    fn exec(&mut self, to: [u8; 20], data: &[u8], apply: bool) -> Result<(), String> {
        let sel: [u8; 4] = data[..4].try_into().unwrap();
        if to == W && sel == deposits::WRAP_DEPOSIT {
            let s_off = arg(data, 0) as usize + 4;
            let s_len = uint(&data[s_off..s_off + 32]) as usize;
            let id = String::from_utf8(data[s_off + 32..s_off + 32 + s_len].to_vec()).unwrap();
            let p_off = arg(data, 1) as usize + 4;
            let n = uint(&data[p_off..p_off + 32]) as usize;
            let proof: Vec<[u8; 32]> = (0..n).map(|i| data[p_off + 32 + 32 * i..p_off + 64 + 32 * i].try_into().unwrap()).collect();
            let txid = arg_word(data, 2);
            let vout = arg(data, 3) as u32;
            let dest: [u8; 20] = arg_word(data, 4)[12..].try_into().unwrap();
            if !frogbuy::verify_proof(&proof, &self.root, &frogbuy::leaf_of(&id)) { return Err("NotInCollection".into()); }
            let key = deposits::outpoint_key(txid, vout);
            if self.minted.contains(&key) { return Err("AlreadyMinted".into()); }
            if apply {
                self.minted.insert(key);
                self.held.insert(frogbuy::leaf_of(&id));
                *self.balances.entry(dest).or_default() += E;
            }
            return Ok(());
        }
        if to == QUEUE && sel == buyqueue::RECORD_ITEM {
            let cost = arg(data, 0);
            if self.balances.get(&QUEUE).copied().unwrap_or(0) < (self.recorded as u128 + 1) * E { return Err("NoWrapped".into()); }
            if cost > self.cap || cost > self.queued { return Err("OverCap".into()); }
            if apply {
                self.queued -= cost;
                self.recorded += 1;
                self.paid += cost;
            }
            return Ok(());
        }
        if to == BOARD && sel == handin::CLAIM {
            let id = arg(data, 0) as usize;
            let s = self.subs.get(id).cloned().ok_or("BadInput")?;
            if s.status != 1 { return Err("WrongStatus".into()); }
            if apply {
                self.subs[id].status = 2;
                let paid = REWARD.min(self.pool);
                self.pool -= paid;
                *self.owed.entry(s.submitter).or_default() += BOND + paid;
            }
            return Ok(());
        }
        if to == BOARD && sel == handin::SLASH {
            let id = arg(data, 0) as usize;
            let s = self.subs.get(id).cloned().ok_or("BadInput")?;
            if s.status != 1 { return Err("WrongStatus".into()); }
            if !self.slashable(&s) { return Err("TooEarly".into()); }
            if apply {
                self.subs[id].status = 3;
                self.pool += BOND;
            }
            return Ok(());
        }
        Err(format!("unknown call to 0x{}", hex::encode(to)))
    }
    fn mine_evm(&mut self) {
        loop {
            let Some(pos) = self.mempool.iter().position(|r| uint(&rlp_items(r)[1]) as u64 == self.nonce) else { return };
            let raw = self.mempool.remove(pos);
            let f = rlp_items(&raw);
            let to: [u8; 20] = f[5].clone().try_into().unwrap();
            let ok = self.exec(to, &f[7], true).is_ok();
            self.calls.push((to, f[7].clone(), ok));
            self.receipts.insert(evmtx::keccak(&raw), ok);
            self.nonce += 1;
        }
    }
}

/// Phase B2: this world's home is the EVM chain (no Solana).
impl etch_autobuyer::solhome::SolIo for World {}

impl FrogIo for World {
    fn now(&self) -> u64 { self.now }
    fn chain_id(&self) -> u64 { 4663 }
    fn wrapper(&self) -> [u8; 20] { W }
    fn fresh(&mut self) {}
    async fn eth_read(&mut self, to: [u8; 20], data: &[u8]) -> Result<Vec<u8>, String> {
        let sel: [u8; 4] = data[..4].try_into().unwrap();
        let b = |x: bool| word(x as u128);
        Ok(match (to, sel) {
            (W, deposits::CUSTODIAN) => aword(&ME),
            (W, deposits::REGISTRY) => aword(&REG),
            (W, deposits::LISTING_ID) => word(0),
            (W, deposits::DEPOSIT_MINTED) => b(self.minted.contains(&arg_word(data, 0))),
            (W, deposits::INSCRIPTION_HELD) => b(self.held.contains(&arg_word(data, 0))),
            (REG, deposits::LISTING) => [word(32), word(1), word(0), word(0), word(0), aword(&[9; 20]), self.root.to_vec()].concat(),
            (QUEUE, buyqueue::QUEUED) => word(self.queued),
            (QUEUE, buyqueue::ITEM_CAP) => word(self.cap),
            (QUEUE, buyqueue::BUFFER) => word(2 * E / 100),
            (QUEUE, buyqueue::ITEMS_RECORDED) => word(self.recorded as u128),
            (QUEUE, buyqueue::HEAD_DEADLINE) => [word((T0 + 7 * 86_400) as u128), word((B0 + 7 * 86_400 / 12) as u128)].concat(),
            (QUEUE, curvebuy::PAYEE) => aword(&ME),
            (QUEUE, curvebuy::WRAPPER) => aword(&W),
            (BOARD, handin::WRAPPER) => aword(&self.board_wrapper),
            (BOARD, handin::PARAMS) => [word(BOND), word(REWARD), word(SLASH_DELAY as u128), word(EXPIRY as u128), word(20 * REWARD), word(64)].concat(),
            (BOARD, handin::OPEN_IDS) => {
                let (start, max) = (arg(data, 0) as usize, arg(data, 1) as usize);
                let ids = self.open_ids();
                let ids: Vec<u64> = ids.into_iter().skip(start).take(max).collect();
                let mut r = word(0x20);
                r.extend(word(ids.len() as u128));
                for i in ids {
                    r.extend(word(i as u128));
                }
                r
            }
            (BOARD, handin::SUBMISSION_OF) => World::encode_sub(self.subs.get(arg(data, 0) as usize).ok_or("BadInput")?),
            (BOARD, handin::SLASHABLE) => {
                let s = self.subs.get(arg(data, 0) as usize).ok_or("BadInput")?.clone();
                b(self.slashable(&s))
            }
            _ => return Err(format!("unknown read 0x{} {}", hex::encode(to), hex::encode(sel))),
        })
    }
    async fn clock(&mut self) -> Result<Clock, String> { Ok(self.clock) }
    async fn our_evm(&mut self) -> Result<[u8; 20], String> { Ok(ME) }
    async fn evm_nonce(&mut self) -> Result<u64, String> { Ok(self.nonce) }
    async fn base_fee(&mut self) -> Result<u128, String> { Ok(1_000_000) }
    async fn estimate_gas(&mut self, to: [u8; 20], _value: u128, data: &[u8]) -> Result<u64, String> {
        self.exec(to, data, false).map(|_| 100_000)
    }
    async fn sign_evm(&mut self, _hash: [u8; 32]) -> Result<(u8, [u8; 32], [u8; 32]), String> { Ok((0, [1; 32], [2; 32])) }
    async fn send_evm(&mut self, raw: &[u8]) -> Result<(), String> {
        let saved = self.saved.as_ref().and_then(|s| s.pending.as_ref()).map(|p| p.raw.clone());
        assert_eq!(saved.as_deref(), Some(raw), "sent EVM bytes that were not saved first");
        if !self.receipts.contains_key(&evmtx::keccak(raw)) && !self.mempool.iter().any(|r| r == raw) {
            self.mempool.push(raw.to_vec());
        }
        Ok(())
    }
    async fn evm_seen(&mut self, hash: [u8; 32]) -> EvmView {
        match self.receipts.get(&hash) {
            Some(ok) => EvmView::Mined { success: *ok },
            None if self.mempool.iter().any(|r| evmtx::keccak(r) == hash) => EvmView::Pending,
            None => EvmView::NotYet,
        }
    }
    async fn float_script(&mut self) -> Result<Vec<u8>, String> { Ok(float()) }
    async fn custody_script(&mut self) -> Result<Vec<u8>, String> { Ok(custody()) }
    async fn locate(&mut self, i: &str) -> Result<Option<Location>, String> { Ok(self.loc.get(i).cloned()) }
    async fn contents(&mut self, o: &Outpoint) -> Result<Option<Contents>, String> { Ok(self.contents.get(o).cloned()) }
    async fn output(&mut self, o: &Outpoint) -> Result<(Output, bool), String> {
        self.outputs.get(o).cloned().ok_or_else(|| format!("no output {}", o.text()))
    }
    async fn outspend(&mut self, o: &Outpoint) -> Result<Option<([u8; 32], bool)>, String> { Ok(self.outspends.get(o).copied()) }
    async fn tx_status(&mut self, txid: [u8; 32]) -> BtcSeen {
        match self.txs.get(&txid) {
            None => BtcSeen::NotYet,
            Some(BtcTx { height: Some(h), hash: Some(b) }) => BtcSeen::Confirmed { height: *h, block_hash: b.clone() },
            Some(_) => BtcSeen::Mempool,
        }
    }
    async fn tip(&mut self) -> Result<u64, String> { Ok(self.tip) }
    async fn block_hash(&mut self, h: u64) -> Result<String, String> { self.blocks.get(&h).cloned().ok_or("no block".into()) }
    async fn btc_send(&mut self, raw: &[u8]) -> Result<(), String> {
        let txid = display(parse_signed(raw).map_err(|e| e.to_string())?.txid);
        self.btc_sent.push(raw.to_vec());
        self.txs.entry(txid).or_insert(BtcTx { height: None, hash: None });
        Ok(())
    }
    async fn custody_utxos(&mut self) -> Result<Vec<Utxo>, String> { Ok(self.utxos.clone()) }
    async fn tx_view(&mut self, txid: [u8; 32]) -> Result<TxView, String> {
        let t = self.txs.get(&txid).ok_or("no such transaction")?;
        Ok(TxView { txid, inputs: vec![], outputs: vec![deposits::OutView { value: 10_000, script: custody() }], confirmed: t.height.is_some(),
            height: t.height, block_hash: t.hash.clone() })
    }
    async fn fee_rate(&mut self) -> Result<u64, String> { Ok(5) }
    async fn rate(&mut self) -> Result<u64, String> { Ok(RATE) }
    async fn input_clean(&mut self, _o: Outpoint) -> Result<(), String> { Ok(()) }
    async fn sign_float(&mut self, _h: [u8; 32]) -> Result<Vec<u8>, String> { self.float_signs += 1; Ok(vec![0x11; 64]) }
    async fn sign_custody(&mut self, _h: [u8; 32]) -> Result<Vec<u8>, String> { Ok(vec![0x22; 64]) }
    fn take_offer(&mut self, frog: u64) -> Option<Offer> { self.offers.remove(&frog) }
    fn save(&mut self, st: &FrogState) { self.saved = Some(st.clone()); }
    fn save_handins(&mut self, hs: &HandinState) { self.saved_handins = Some(hs.clone()); }
    fn with_caps<R>(&mut self, f: impl FnOnce(&custodycap::CapPolicy, &mut custodycap::Book) -> R) -> R { f(&self.caps, &mut self.book) }
    fn outflow_check(&mut self, key: &str, collection: &str, flow: outflow::Flow, amount: u128) -> Result<(), String> {
        self.asked.push((key.to_string(), flow, amount));
        let Some(l) = self.limits.clone() else { return Ok(()) };
        match self.outflow.check(&l, key, collection, flow, amount, self.now) {
            outflow::Verdict::Allow => Ok(()),
            outflow::Verdict::Wait { why, .. } => Err(format!("waits (outflow limit): {why}")),
            outflow::Verdict::Frozen(c) => Err(format!("refused: custody is frozen: {c:?}")),
        }
    }
    fn outflow_record(&mut self, key: &str, collection: &str, flow: outflow::Flow, amount: u128) {
        self.outflow.record(key, collection, flow, amount, self.now);
    }
    fn outbound_policy(&self, custody_script: &[u8]) -> outbound::BtcPolicy {
        outbound::BtcPolicy { custody_script: custody_script.to_vec(), max_fee_sats: outbound::MAX_FEE_SATS, require_finalized: true,
            require_event: true }
    }
    fn paused(&self) -> bool { false }
}

// ---- helpers ------------------------------------------------------------------------------------------------------------------------

fn tick(w: &mut World, st: &mut FrogState, hs: &mut HandinState) -> Vec<String> {
    tick_busy(w, st, hs, false)
}

fn tick_busy(w: &mut World, st: &mut FrogState, hs: &mut HandinState, others_pending: bool) -> Vec<String> {
    let lines = block_on(frogflow::tick_full(w, st, &mut FloorState::default(), hs, others_pending, &[]));
    for l in &lines {
        println!("{l}");
    }
    assert_eq!(w.saved_handins.as_ref(), Some(&*hs), "the hand-in state is saved before the tick ends");
    lines
}

fn state() -> (FrogState, HandinState) {
    let st = FrogState { sinks: vec![SinkRec { kind: SinkKind::Queue, sink: QUEUE, coin: [0; 20], view: None, refused: None,
        drove_refunds_at: None, drove_settle: false }], ..Default::default() };
    let mut hs = HandinState::default();
    hs.add_board(BOARD).unwrap();
    (st, hs)
}

fn phase(st: &FrogState, id: u64) -> FPhase {
    st.frogs.iter().find(|f| f.id == id).unwrap().phase.clone()
}

fn step(st: &FrogState) -> Option<FStep> {
    st.pending.as_ref().map(|p| p.step.clone())
}

fn seller(tag: u8) -> Outpoint {
    Outpoint { txid: [tag; 32], vout: 0 }
}

/// A world with a collection of four frogs, frog 1 listed at `seller(0xe1)`, and the queue wanting one item (the first tick).
fn wanting() -> (World, FrogState, HandinState, Vec<Vec<[u8; 32]>>) {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2), insc(3), insc(4)]);
    let (mut st, mut hs) = state();
    w.list(&insc(1), seller(0xe1));
    tick(&mut w, &mut st, &mut hs);
    assert!(matches!(phase(&st, 0), FPhase::Wanted { .. }));
    assert_eq!(hs.board(&BOARD).unwrap().params.map(|p| p.reward_wei), Some(REWARD), "the board's params were read");
    (w, st, hs, proofs)
}

/// From `Signed` to `Held`, then the mint and the report, to `Settled`. The claim comes after.
fn confirm_and_settle(w: &mut World, st: &mut FrogState, hs: &mut HandinState, frog: u64) {
    w.mine_btc(1);
    tick(w, st, hs);
    assert!(matches!(phase(st, frog), FPhase::Sent { .. }));
    w.mine_btc(5);
    tick(w, st, hs);
    assert!(matches!(phase(st, frog), FPhase::Held { .. }), "{:?}", phase(st, frog));
    assert_eq!(step(st), Some(FStep::Mint { frog }), "the mint goes before any claim");
    w.mine_evm();
    tick(w, st, hs);
    assert_eq!(step(st), Some(FStep::Report { frog }));
    w.mine_evm();
    tick(w, st, hs);
    assert!(matches!(phase(st, frog), FPhase::Settled { .. }));
}

// ---- the tests ----------------------------------------------------------------------------------------------------------------------

#[test]
fn a_valid_hand_in_is_bought_confirmed_and_claimed_and_the_bot_is_credited_bond_plus_reward() {
    let (mut w, mut st, mut hs, proofs) = wanting();
    let (bytes, to_sign) = w.psbt_for(1, seller(0xe1), PRICE, false);
    let id = w.hand_in(&mut hs, BOT, &bytes, &to_sign, PRICE + 3_000, &insc(1), proofs[0].clone(), true);
    let lines = tick(&mut w, &mut st, &mut hs);
    assert!(lines.iter().any(|l| l.contains("hand-in 0") && l.contains("signed, saved and sent")), "{lines:?}");
    assert!(matches!(phase(&st, 0), FPhase::Signed { .. }));
    assert_eq!(hs.verdict_of(&BOARD, id), Some(&Verdict::Bought { frog: 0 }));
    confirm_and_settle(&mut w, &mut st, &mut hs, 0);
    // the claim is the fifth group: it goes out once nothing more urgent is owed
    assert_eq!(step(&st), Some(FStep::Claim { board: BOARD, id }));
    assert!(w.asked.iter().any(|(k, f, a)| k == "claim:handin:0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b:0" && *f == outflow::Flow::Value && *a == REWARD),
        "the reward counts against the day's value limit (H3): {:?}", w.asked);
    w.mine_evm();
    tick(&mut w, &mut st, &mut hs);
    assert_eq!(hs.verdict_of(&BOARD, id), Some(&Verdict::Claimed));
    assert_eq!(w.subs[id as usize].status, 2, "Claimed on the board");
    assert_eq!(w.owed[&BOT], BOND + REWARD, "bond back plus the reward");
    assert_eq!(w.pool, E - REWARD);
    // the next read no longer lists it; nothing is claimed twice
    tick(&mut w, &mut st, &mut hs);
    assert!(!hs.seen(&BOARD, id).unwrap().open);
    assert!(st.pending.is_none());
    assert_eq!(w.calls.iter().filter(|(to, d, _)| *to == BOARD && d[..4] == handin::CLAIM).count(), 1);
}

#[test]
fn the_operators_offer_still_comes_first() {
    let (mut w, mut st, mut hs, proofs) = wanting();
    let (bytes, to_sign) = w.psbt_for(1, seller(0xe1), PRICE, false);
    let id = w.hand_in(&mut hs, BOT, &bytes, &to_sign, PRICE + 3_000, &insc(1), proofs[0].clone(), true);
    let (op_bytes, op_sign) = w.psbt_for(9, seller(0xe1), PRICE - 100, false);
    w.offers.insert(0, Offer { inscription: insc(1), psbt: op_bytes, to_sign: op_sign, proof: proofs[0].clone() });
    let lines = tick(&mut w, &mut st, &mut hs);
    assert!(lines.iter().any(|l| l.contains("frog 0:") && l.contains("signed, saved and sent") && !l.contains("hand-in")), "{lines:?}");
    assert!(hs.verdict_of(&BOARD, id).is_none(), "the hand-in was not tried: no verdict, its bond back at expiry");
}

#[test]
fn an_invalid_hand_in_is_slashed_after_the_delay_on_both_clocks_and_never_bought() {
    let (mut w, mut st, mut hs, proofs) = wanting();
    // a runestone output: the judge refuses it
    let (bytes, to_sign) = w.psbt_for(1, seller(0xe1), PRICE, true);
    let id = w.hand_in(&mut hs, BOT, &bytes, &to_sign, PRICE + 3_000, &insc(1), proofs[0].clone(), true);
    let lines = tick(&mut w, &mut st, &mut hs);
    assert!(lines.iter().any(|l| l.contains("INVALID") && l.contains("OpReturn")), "{lines:?}");
    assert!(matches!(hs.verdict_of(&BOARD, id), Some(Verdict::Invalid { why }) if why.contains("OpReturn")));
    assert!(matches!(phase(&st, 0), FPhase::Wanted { .. }));
    assert!(w.btc_sent.is_empty(), "nothing signed");
    // not slashable yet: no transaction of ours goes out
    tick(&mut w, &mut st, &mut hs);
    assert!(st.pending.is_none());
    // the sequencer's clock alone does not make it slashable
    w.now += SLASH_DELAY;
    w.clock.time += SLASH_DELAY;
    tick(&mut w, &mut st, &mut hs);
    assert!(st.pending.is_none(), "the parent chain has not moved: the board says not yet");
    w.clock.parent_block += SLASH_DELAY / 12;
    tick(&mut w, &mut st, &mut hs);
    assert_eq!(step(&st), Some(FStep::Slash { board: BOARD, id }));
    w.mine_evm();
    tick(&mut w, &mut st, &mut hs);
    assert_eq!(hs.verdict_of(&BOARD, id), Some(&Verdict::Slashed));
    assert_eq!(w.subs[id as usize].status, 3);
    assert_eq!(w.pool, E + BOND, "the bond went to the pool");
    assert!(!w.owed.contains_key(&BOT));
    // it is never tried again
    tick(&mut w, &mut st, &mut hs);
    assert!(matches!(phase(&st, 0), FPhase::Wanted { .. }));
    assert!(w.btc_sent.is_empty());
}

#[test]
fn a_hand_in_that_lies_about_its_ask_is_invalid() {
    let (mut w, mut st, mut hs, proofs) = wanting();
    let (bytes, to_sign) = w.psbt_for(1, seller(0xe1), PRICE, false);
    // it spends 903,000 sats and says 800,000, to be picked first
    let id = w.hand_in(&mut hs, BOT, &bytes, &to_sign, 800_000, &insc(1), proofs[0].clone(), true);
    let lines = tick(&mut w, &mut st, &mut hs);
    assert!(lines.iter().any(|l| l.contains("INVALID") && l.contains("more than the 800000 it said")), "{lines:?}");
    assert!(matches!(hs.verdict_of(&BOARD, id), Some(Verdict::Invalid { .. })));
    assert!(w.btc_sent.is_empty());
}

#[test]
fn the_cheapest_valid_hand_in_is_bought_and_a_dearer_one_under_the_cap_is_ignored() {
    let (mut w, mut st, mut hs, proofs) = wanting();
    w.list(&insc(2), seller(0xe2));
    // the dear one first, a bot's own frog priced just under the cap (≈1.25M sats); then a cheaper listing of another frog
    let (dear, s1) = w.psbt_for(1, seller(0xe1), 1_197_000, false);
    let dear_id = w.hand_in(&mut hs, BOT, &dear, &s1, 1_200_000, &insc(1), proofs[0].clone(), true);
    let (cheap, s2) = w.psbt_for(2, seller(0xe2), PRICE, false);
    let cheap_id = w.hand_in(&mut hs, BOT2, &cheap, &s2, PRICE + 3_000, &insc(2), proofs[1].clone(), true);
    // and one over every cap: never tried
    let (over, s3) = w.psbt_for(3, seller(0xe1), 1_500_000, false);
    let over_id = w.hand_in(&mut hs, BOT, &over, &s3, 1_503_000, &insc(1), proofs[0].clone(), true);
    let lines = tick(&mut w, &mut st, &mut hs);
    assert!(lines.iter().any(|l| l.contains(&format!("hand-in {cheap_id}")) && l.contains("signed")), "{lines:?}");
    assert_eq!(hs.verdict_of(&BOARD, cheap_id), Some(&Verdict::Bought { frog: 0 }));
    assert!(hs.verdict_of(&BOARD, dear_id).is_none(), "not tried: still open, refundable at expiry");
    assert!(hs.verdict_of(&BOARD, over_id).is_none());
    let FPhase::Signed { buy } = phase(&st, 0) else { panic!() };
    assert_eq!(buy.inscription, insc(2));
    confirm_and_settle(&mut w, &mut st, &mut hs, 0);
    assert_eq!(step(&st), Some(FStep::Claim { board: BOARD, id: cheap_id }));
    w.mine_evm();
    tick(&mut w, &mut st, &mut hs);
    assert_eq!(w.owed.get(&BOT2), Some(&(BOND + REWARD)));
    assert!(!w.owed.contains_key(&BOT), "the dear one earned nothing");
    // the queue no longer wants an item: the dear one stays open until its submitter refunds it
    tick(&mut w, &mut st, &mut hs);
    assert_eq!(st.frogs.len(), 1);
    w.advance(EXPIRY);
    w.refund(dear_id);
    tick(&mut w, &mut st, &mut hs);
    assert!(!hs.seen(&BOARD, dear_id).unwrap().open);
    assert_eq!(w.owed[&BOT], BOND, "its bond, no reward");
}

#[test]
fn a_dead_listing_is_gone_not_slashed_and_the_next_hand_in_is_tried() {
    let (mut w, mut st, mut hs, proofs) = wanting();
    w.list(&insc(2), seller(0xe2));
    let (dead, s1) = w.psbt_for(1, seller(0xe1), PRICE - 1_000, false);
    let dead_id = w.hand_in(&mut hs, BOT, &dead, &s1, PRICE + 2_000, &insc(1), proofs[0].clone(), true);
    let (live, s2) = w.psbt_for(2, seller(0xe2), PRICE, false);
    let live_id = w.hand_in(&mut hs, BOT2, &live, &s2, PRICE + 3_000, &insc(2), proofs[1].clone(), true);
    // frog 1 was sold elsewhere before the tick
    w.outputs.get_mut(&seller(0xe1)).unwrap().1 = true;
    let lines = tick(&mut w, &mut st, &mut hs);
    assert!(lines.iter().any(|l| l.contains(&format!("hand-in {dead_id} not fillable"))), "{lines:?}");
    assert!(matches!(hs.verdict_of(&BOARD, dead_id), Some(Verdict::Gone { .. })));
    assert!(matches!(phase(&st, 0), FPhase::Wanted { .. }), "one hand-in tried a tick");
    tick(&mut w, &mut st, &mut hs);
    assert_eq!(hs.verdict_of(&BOARD, live_id), Some(&Verdict::Bought { frog: 0 }));
    // the dead one is never slashed, whatever the clocks say
    w.advance(2 * SLASH_DELAY);
    tick(&mut w, &mut st, &mut hs);
    assert!(hs.slashes_due().is_empty());
}

#[test]
fn a_submission_whose_bytes_never_arrive_is_slashed() {
    let (mut w, mut st, mut hs, proofs) = wanting();
    let (bytes, to_sign) = w.psbt_for(1, seller(0xe1), PRICE, false);
    let id = w.hand_in(&mut hs, BOT, &bytes, &to_sign, PRICE + 3_000, &insc(1), proofs[0].clone(), false);
    let lines = tick(&mut w, &mut st, &mut hs);
    assert!(lines.iter().any(|l| l.contains("no offer yet")), "{lines:?}");
    assert!(hs.verdict_of(&BOARD, id).is_none());
    w.advance(SLASH_DELAY);
    tick(&mut w, &mut st, &mut hs);
    assert!(matches!(hs.verdict_of(&BOARD, id), Some(Verdict::Invalid { why }) if why.contains("never arrived")));
    assert_eq!(step(&st), Some(FStep::Slash { board: BOARD, id }));
    w.mine_evm();
    tick(&mut w, &mut st, &mut hs);
    assert_eq!(w.subs[id as usize].status, 3);
}

#[test]
fn bytes_delivered_before_the_read_are_buffered_and_matched_at_the_read() {
    let (mut w, mut st, mut hs, proofs) = wanting();
    let (bytes, to_sign) = w.psbt_for(1, seller(0xe1), PRICE, false);
    // the bot sends the bytes first, then submits on chain (or the canister has not ticked since its submission)
    hs.intake(w.now, bytes.clone(), to_sign.iter().map(|i| *i as u32).collect(), proofs[0].clone()).unwrap();
    assert_eq!((hs.unmatched.len(), hs.bytes.len()), (1, 0));
    let id = w.submit(BOT, handin::offer_hash(&bytes), PRICE + 3_000, &insc(1));
    tick(&mut w, &mut st, &mut hs);
    assert_eq!((hs.unmatched.len(), hs.bytes.len()), (0, 1));
    assert_eq!(hs.verdict_of(&BOARD, id), Some(&Verdict::Bought { frog: 0 }), "matched at the read and bought in the same tick");
}

#[test]
fn the_board_is_read_a_bounded_number_of_new_submissions_a_tick() {
    let (mut w, mut st, mut hs, _) = wanting();
    for i in 0..20u8 {
        w.submit(BOT, [i + 1; 32], 2_000_000 + i as u64, &insc(1));
    }
    tick(&mut w, &mut st, &mut hs);
    assert_eq!(hs.seen.len(), handin::MAX_SUBMISSIONS_READ_PER_TICK);
    tick(&mut w, &mut st, &mut hs);
    assert_eq!(hs.seen.len(), 20);
}

#[test]
fn a_reorganisation_after_held_stops_the_claim_until_six_again() {
    let (mut w, mut st, mut hs, proofs) = wanting();
    let (bytes, to_sign) = w.psbt_for(1, seller(0xe1), PRICE, false);
    let id = w.hand_in(&mut hs, BOT, &bytes, &to_sign, PRICE + 3_000, &insc(1), proofs[0].clone(), true);
    tick(&mut w, &mut st, &mut hs);
    let FPhase::Signed { buy } = phase(&st, 0) else { panic!() };
    w.mine_btc(6);
    // held while the EVM side is busy: no mint and no claim yet
    tick_busy(&mut w, &mut st, &mut hs, true);
    tick_busy(&mut w, &mut st, &mut hs, true);
    assert!(matches!(phase(&st, 0), FPhase::Held { .. }));
    assert_eq!(hs.claims_due(&st.frogs), vec![(BOARD, id, 0)]);
    // the purchase's block is replaced: the mint re-checks depth and sends the frog back to waiting; no claim goes out
    w.reorg_out(buy.outpoint.txid);
    let lines = tick(&mut w, &mut st, &mut hs);
    assert!(lines.iter().any(|l| l.contains("reorganisation")), "{lines:?}");
    assert!(st.pending.is_none(), "nothing sent: not a claim either");
    assert!(w.calls.iter().all(|(to, _, _)| *to != BOARD));
    // it confirms again: six deep, held, minted, reported, and only then claimed
    w.mine_btc(6);
    tick(&mut w, &mut st, &mut hs);
    tick(&mut w, &mut st, &mut hs);
    assert!(matches!(phase(&st, 0), FPhase::Held { .. }), "{:?}", phase(&st, 0));
    assert_eq!(step(&st), Some(FStep::Mint { frog: 0 }));
    w.mine_evm();
    tick(&mut w, &mut st, &mut hs);
    w.mine_evm();
    tick(&mut w, &mut st, &mut hs);
    assert_eq!(step(&st), Some(FStep::Claim { board: BOARD, id }));
    w.mine_evm();
    tick(&mut w, &mut st, &mut hs);
    assert_eq!(hs.verdict_of(&BOARD, id), Some(&Verdict::Claimed));
}

#[test]
fn the_listing_vanished_after_signing_earns_nothing_and_is_not_slashed() {
    let (mut w, mut st, mut hs, proofs) = wanting();
    let (bytes, to_sign) = w.psbt_for(1, seller(0xe1), PRICE, false);
    let id = w.hand_in(&mut hs, BOT, &bytes, &to_sign, PRICE + 3_000, &insc(1), proofs[0].clone(), true);
    tick(&mut w, &mut st, &mut hs);
    let FPhase::Signed { buy } = phase(&st, 0) else { panic!() };
    // the submitter (the seller) spent the outpoint another way first
    w.txs.remove(&buy.outpoint.txid);
    w.outspends.insert(seller(0xe1), ([0x99; 32], true));
    tick(&mut w, &mut st, &mut hs);
    assert!(matches!(phase(&st, 0), FPhase::Failed { .. }));
    tick(&mut w, &mut st, &mut hs);
    assert!(matches!(hs.verdict_of(&BOARD, id), Some(Verdict::Gone { why }) if why.contains("failed")));
    assert!(hs.claims_due(&st.frogs).is_empty() && hs.slashes_due().is_empty());
    assert!(st.frogs.iter().any(|f| f.id == 1 && matches!(f.phase, FPhase::Wanted { .. })), "a new record wants again");
}

#[test]
fn a_submitter_who_refunds_before_the_claim_is_not_paid_and_the_revert_is_read_from_the_board() {
    let (mut w, mut st, mut hs, proofs) = wanting();
    let (bytes, to_sign) = w.psbt_for(1, seller(0xe1), PRICE, false);
    let id = w.hand_in(&mut hs, BOT, &bytes, &to_sign, PRICE + 3_000, &insc(1), proofs[0].clone(), true);
    tick(&mut w, &mut st, &mut hs);
    confirm_and_settle(&mut w, &mut st, &mut hs, 0);
    assert_eq!(step(&st), Some(FStep::Claim { board: BOARD, id }));
    // the claim is out; before it mines the submitter refunds (expiry passed on the board's clocks)
    w.advance(EXPIRY);
    w.refund(id);
    w.mine_evm();
    let lines = tick(&mut w, &mut st, &mut hs);
    assert!(lines.iter().any(|l| l.contains("REVERTED") && l.contains("took the bond back")), "{lines:?}");
    assert!(matches!(hs.verdict_of(&BOARD, id), Some(Verdict::Gone { .. })));
    assert_eq!(w.owed[&BOT], BOND);
    tick(&mut w, &mut st, &mut hs);
    assert!(st.pending.is_none(), "not claimed again");
}

#[test]
fn a_hand_in_too_near_its_expiry_is_not_started() {
    let (mut w, mut st, mut hs, proofs) = wanting();
    let (bytes, to_sign) = w.psbt_for(1, seller(0xe1), PRICE, false);
    let id = w.hand_in(&mut hs, BOT, &bytes, &to_sign, PRICE + 3_000, &insc(1), proofs[0].clone(), true);
    // read, but not tried: the queue is read first and the frog wanted before the board... so make the submission old
    w.subs[id as usize].at = T0 - EXPIRY + 4 * 3_600;
    w.subs[id as usize].at_block = B0 - (EXPIRY - 4 * 3_600) / 12;
    let lines = tick(&mut w, &mut st, &mut hs);
    assert!(lines.iter().any(|l| l.contains("no offer yet")), "{lines:?}");
    assert!(hs.verdict_of(&BOARD, id).is_none());
    assert!(w.btc_sent.is_empty());
}

#[test]
fn a_board_whose_wrapper_is_not_ours_is_never_read() {
    let mut w = World::new();
    w.board_wrapper = [0xdd; 20];
    let (mut st, mut hs) = state();
    let lines = tick(&mut w, &mut st, &mut hs);
    assert!(lines.iter().any(|l| l.contains("never read") && l.contains("not ours")), "{lines:?}");
    assert!(hs.board(&BOARD).unwrap().refused.is_some());
    w.submit(BOT, [1; 32], 1, &insc(1));
    tick(&mut w, &mut st, &mut hs);
    assert!(hs.seen.is_empty());
}

#[test]
fn the_days_value_limit_holds_the_claim_and_a_tick_without_boards_is_the_old_tick() {
    let (mut w, mut st, mut hs, proofs) = wanting();
    let (bytes, to_sign) = w.psbt_for(1, seller(0xe1), PRICE, false);
    let id = w.hand_in(&mut hs, BOT, &bytes, &to_sign, PRICE + 3_000, &insc(1), proofs[0].clone(), true);
    tick(&mut w, &mut st, &mut hs);
    confirm_and_settle(&mut w, &mut st, &mut hs, 0);
    // the claim went out under no limits; with a value limit already spent it would have waited
    assert_eq!(step(&st), Some(FStep::Claim { board: BOARD, id }));
    let mut l = outflow::Limits::stage0();
    l.global_value_wei = REWARD - 1;
    let mut o = outflow::Outflow::default();
    assert!(matches!(o.check(&l, "claim:handin:x:0", &custodycap::btc_key(&W), outflow::Flow::Value, REWARD, T0), outflow::Verdict::Wait { .. }));
    // and a world with no board behaves as before: `tick_with` is `tick_full` with nothing
    let mut w2 = World::new();
    w2.collection(&[insc(1), insc(2), insc(3), insc(4)]);
    let mut st2 = FrogState { sinks: st.sinks.clone(), ..Default::default() };
    let lines = block_on(frogflow::tick_with(&mut w2, &mut st2, &mut FloorState::default(), false, &[]));
    assert!(lines.iter().any(|l| l.contains("waiting for an offer")), "{lines:?}");
    assert!(w2.saved_handins.as_ref().is_none_or(|h| h.boards.is_empty()));
}

/// Simulation finding (ops/rehearsal/sim, 2026-10-09): a hand-in whose seller input carries NO final witness (the shape of a
/// marketplace bid when the marketplace keeps the seller's signature server-side) passes `frogbuy::judge` — the judge never asks
/// for the seller's witness — so every tick signs the float's inputs with the threshold key and only then `psbt::finalize`
/// refuses it as `Later`. The hand-in is never ruled Invalid (no slash), and each tick repeats the signing calls. Fixed (review
/// S-01): the judge refuses it as `NotFinal`, so it is ruled Invalid before the float key is asked for anything.
#[test]
fn a_hand_in_without_the_sellers_witness_is_invalid_before_anything_is_signed() {
    let (mut w, mut st, mut hs, proofs) = wanting();
    let (bytes, to_sign) = w.psbt_for(1, seller(0xe1), PRICE, false);
    // strip the seller's final witness: the same PSBT with input 2 unsigned
    let p = Psbt::parse(&bytes).unwrap();
    let unsigned = frogbuy::psbt_bytes(&p, &[None, None, None, None]);
    let id = w.hand_in(&mut hs, BOT, &unsigned, &to_sign, PRICE + 3_000, &insc(1), proofs[0].clone(), true);
    let l1 = tick(&mut w, &mut st, &mut hs);
    let l2 = tick(&mut w, &mut st, &mut hs);
    let waits = l1.iter().chain(l2.iter()).filter(|l| l.contains("waits") && l.contains("carries no witness")).count();
    assert_eq!(waits, 0, "signed then refused at finalize, tick after tick: {l1:?} {l2:?}");
    assert!(matches!(hs.verdict_of(&BOARD, id), Some(Verdict::Invalid { .. })), "{:?}", hs.verdict_of(&BOARD, id));
    assert!(l1.iter().any(|l| l.contains("INVALID, nothing signed") && l.contains("NotFinal(2)")), "{l1:?}");
    assert_eq!(w.float_signs, 0, "the float key is never asked: {l1:?} {l2:?}");
    assert!(w.btc_sent.is_empty());
}
