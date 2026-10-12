//! The buyer's loop against a fake world (`docs/v2/buyer-loop.md`). The world is a small chain that EXECUTES our signed
//! transactions against a model of the vault -- announce, pull, settleRune, returnFunds, with their status rules -- plus a
//! Bitcoin that holds our sats and our transaction, a venue and a bridge from fixtures, and a stable memory. It can lag,
//! disagree, revert, and have the canister die between any two calls. Nothing here touches a network.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use etch_autobuyer::bridge::{Bridge, BridgeQuote, Fixture as BridgeFixture};
use etch_autobuyer::evmtx::{self, VaultPurchase};
use etch_autobuyer::policy;
use etch_autobuyer::looper::{self, BtcView, EvmView, Io};
use etch_autobuyer::psbt::Quote;
use etch_autobuyer::runestone::RuneId;
use etch_autobuyer::state::{EvmStep, Order, Phase, Purchase, Stuck};
use etch_autobuyer::venue::{Fixture as VenueFixture, Venue};

fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

const OUR: &[u8] = &[0x51, 0x20, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1];
const WINNER: &[u8] = &[0x51, 0x20, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2];
const SELLER: &[u8] = &[0x00, 0x14, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7];
const RUNE: RuneId = RuneId { block: 840_000, tx: 3 };
const ONE_ETH: u128 = 1_000_000_000_000_000_000;
const VAULT: [u8; 20] = [0xaa; 20];
const BRIDGE: [u8; 20] = [0xb4; 20];
const HOUR: u64 = 3600;

// ---- a PSBT the way a venue would return one (as tests/e2e.rs builds them) -------------------------------------------------

fn leb(mut v: u128) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return out;
        }
        out.push(b | 0x80);
    }
}
fn stone(amount: u128, to: u128) -> Vec<u8> {
    let mut p = leb(0);
    p.extend(leb(RUNE.block as u128));
    p.extend(leb(RUNE.tx as u128));
    p.extend(leb(amount));
    p.extend(leb(to));
    let mut s = vec![0x6a, 0x5d, p.len() as u8];
    s.extend(p);
    s
}
fn ser_out(value: u64, script: &[u8]) -> Vec<u8> {
    let mut v = value.to_le_bytes().to_vec();
    v.push(script.len() as u8);
    v.extend_from_slice(script);
    v
}
fn psbt_bytes(in_value: u64, outs: &[(u64, Vec<u8>)]) -> Vec<u8> {
    let mut tx = 2i32.to_le_bytes().to_vec();
    tx.push(1);
    tx.extend_from_slice(&[1u8; 32]);
    tx.extend_from_slice(&0u32.to_le_bytes());
    tx.push(0);
    tx.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
    tx.push(outs.len() as u8);
    for (v, s) in outs {
        tx.extend(ser_out(*v, s));
    }
    tx.extend_from_slice(&0u32.to_le_bytes());
    let mut p = b"psbt\xff".to_vec();
    p.extend_from_slice(&[1, 0x00, tx.len() as u8]);
    p.extend_from_slice(&tx);
    p.push(0);
    let u = ser_out(in_value, OUR);
    p.extend_from_slice(&[1, 0x01, u.len() as u8]);
    p.extend_from_slice(&u);
    p.push(0);
    for _ in outs {
        p.push(0);
    }
    p
}
fn quote() -> Quote {
    Quote { rune: RUNE, units: 1_000, max_spend_sats: 50_000, max_fee_sats: 5_000, recipient_script: WINNER.to_vec(), our_script: OUR.to_vec() }
}
fn venue() -> VenueFixture {
    VenueFixture {
        quote: quote(),
        psbt: psbt_bytes(100_000, &[(0, stone(1_000, 1)), (546, WINNER.to_vec()), (40_000, SELLER.to_vec()), (58_454, OUR.to_vec())]),
        to_sign: vec![0],
    }
}
fn bridge() -> BridgeFixture {
    BridgeFixture { quote: BridgeQuote { send_wei: ONE_ETH, expect_sats: 100_000, to: BRIDGE, calldata: vec![9, 9], recipient_script: OUR.to_vec() } }
}

// ---- our signed transactions, read back by the fake chain ------------------------------------------------------------------

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
    assert_eq!(b[0], 0x02, "EIP-1559");
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
struct Sent { nonce: u64, to: [u8; 20], value: u128, data: Vec<u8> }
fn read_tx(raw: &[u8]) -> Sent {
    let f = rlp_items(raw);
    Sent { nonce: uint(&f[1]) as u64, to: f[5].clone().try_into().unwrap(), value: uint(&f[6]), data: f[7].clone() }
}
fn arg(data: &[u8], n: usize) -> u128 {
    u128::from_be_bytes(data[4 + 32 * n + 16..4 + 32 * n + 32].try_into().unwrap())
}

// ---- the world -------------------------------------------------------------------------------------------------------------

#[derive(Default)]
struct World {
    now: u64,                               // the canister's clock
    chain_time: u64,                        // the vault's clock
    vault: Vec<VaultPurchase>,
    nonce: u64,
    mempool: Vec<Vec<u8>>,
    receipts: HashMap<[u8; 32], bool>,
    balance: u128,
    base_fee: u128,
    btc_sats: u64,
    btc_known: HashSet<[u8; 32]>,
    btc_confirmed: HashSet<[u8; 32]>,
    btc_sent: Vec<Vec<u8>>,
    delivered: Option<u128>,
    saved: Option<Purchase>,
    saves: usize,
    sent: Vec<Vec<u8>>,
    signed: usize,
    venue: Option<VenueFixture>,
    bridge: Option<BridgeFixture>,
    // failure switches
    die_on_send: bool,        // the canister dies inside the send: nothing reaches the chain, the save stands
    disagree: bool,           // the providers disagree about the vault
    lag: bool,                // the chain has mined it but the read-back does not show it yet
    revert_next: bool,        // the next mined transaction reverts though its estimate passed
    // the invariant: nothing is sent that was not first saved as pending with the same bytes
    violations: Vec<String>,
    // docs/v2/trust-points.md §1: what the registry says the listing pays (None: WINNER's address) and whether it is paused,
    // and the reference cap on a quote's spend (None: 60,000 sats for the released ETH)
    listing_btc: Option<String>,
    listing_paused: bool,
    cap: Option<u64>,
    // docs/v3/wrapper.md, "Custody on its own key": what ord says our PSBT input carries (None: clean), and how often it was asked
    ord_says: Option<&'static str>,
    ord_asked: usize,
    taproot_signed: usize,
}

impl World {
    fn new() -> Self {
        World { now: 1_000_000, chain_time: 1_000_000, balance: 0, base_fee: 10_000_000, delivered: Some(0),
            venue: Some(venue()), bridge: Some(bridge()), ..Default::default() }
    }

    /// Would `s` succeed against the vault as it stands? The same rules `mine` applies.
    fn exec(&mut self, s: &Sent, apply: bool) -> Result<(), String> {
        if s.to == BRIDGE {
            if self.balance < s.value { return Err("insufficient".into()); }
            if apply { self.balance -= s.value; }
            return Ok(());
        }
        if s.to != VAULT { return Err("unknown contract".into()); }
        let sel: [u8; 4] = s.data[..4].try_into().unwrap();
        let t = self.chain_time;
        match sel {
            evmtx::ANNOUNCE => {
                if apply {
                    self.vault.push(VaultPurchase { status: 1, ready_at: t + HOUR, ready_at_block: 0, deadline: 0, late: false,
                        max_eth: arg(&s.data, 0), quantity: arg(&s.data, 1), settled_quantity: 0, returned: 0, cap_ref: "cap".into() });
                }
                Ok(())
            }
            evmtx::PULL => {
                let id = arg(&s.data, 0) as usize;
                let v = self.vault.get_mut(id).ok_or("no purchase")?;
                if v.status != 1 { return Err("WrongStatus".into()); }
                if t < v.ready_at { return Err("TooEarly".into()); }
                if apply { v.status = 2; v.deadline = t + 3 * 24 * HOUR; self.balance += v.max_eth; }
                Ok(())
            }
            evmtx::SETTLE_RUNE => {
                let id = arg(&s.data, 0) as usize;
                let v = self.vault.get_mut(id).ok_or("no purchase")?;
                if v.status != 2 { return Err("WrongStatus".into()); }
                if apply { v.status = 3; v.settled_quantity = arg(&s.data, 1); }
                Ok(())
            }
            evmtx::RETURN_FUNDS => {
                let id = arg(&s.data, 0) as usize;
                let bal = self.balance;
                let v = self.vault.get_mut(id).ok_or("no purchase")?;
                match (v.status, s.value) {
                    (1, 0) => { if apply { v.status = 4; } Ok(()) }
                    (2, x) if x > 0 && v.returned + x <= v.max_eth && x <= bal => {
                        if apply { v.returned += x; if v.returned == v.max_eth { v.status = 4; } self.balance -= x; }
                        Ok(())
                    }
                    _ => Err("BadQuantity".into()),
                }
            }
            _ => Err("unknown selector".into()),
        }
    }

    /// Mine everything mineable in nonce order. A reverted transaction still uses its nonce.
    fn mine(&mut self) {
        loop {
            let Some(pos) = self.mempool.iter().position(|r| read_tx(r).nonce == self.nonce) else { return };
            let raw = self.mempool.remove(pos);
            let hash = evmtx::keccak(&raw);
            if self.receipts.contains_key(&hash) { continue; }
            let s = read_tx(&raw);
            let ok = if self.revert_next { self.revert_next = false; false } else { self.exec(&s, true).is_ok() };
            self.receipts.insert(hash, ok);
            self.nonce += 1;
            self.mempool.retain(|r| read_tx(r).nonce >= self.nonce);
        }
    }
}

impl Io for World {
    fn now(&self) -> u64 { self.now }
    fn chain_id(&self) -> u64 { 31337 }
    fn vault(&self) -> [u8; 20] { VAULT }
    async fn our_script(&mut self) -> Result<Vec<u8>, String> { Ok(OUR.to_vec()) }
    async fn purchase_count(&mut self) -> Result<u64, String> {
        if self.disagree { return Err("providers disagree on eth_call; nothing is believed".into()); }
        Ok(self.vault.len() as u64)
    }
    async fn vault_purchase(&mut self, id: u64) -> Result<VaultPurchase, String> {
        if self.disagree { return Err("providers disagree on eth_call; nothing is believed".into()); }
        self.vault.get(id as usize).cloned().ok_or("execution reverted".into())
    }
    async fn evm_balance(&mut self) -> Result<u128, String> { Ok(self.balance) }
    async fn evm_nonce(&mut self) -> Result<u64, String> { Ok(self.nonce) }
    async fn base_fee(&mut self) -> Result<u128, String> { Ok(self.base_fee) }
    async fn estimate_gas(&mut self, to: [u8; 20], value: u128, data: &[u8]) -> Result<u64, String> {
        self.exec(&Sent { nonce: 0, to, value, data: data.to_vec() }, false).map(|_| 60_000).map_err(|e| format!("execution reverted: {e}"))
    }
    async fn sign_evm(&mut self, _hash: [u8; 32]) -> Result<(u8, [u8; 32], [u8; 32]), String> {
        self.signed += 1;
        Ok((0, [0x11; 32], [0x22; 32]))
    }
    async fn send_evm(&mut self, raw: &[u8]) -> Result<(), String> {
        let saved = self.saved.as_ref().and_then(|p| p.pending_evm.as_ref()).map(|pe| pe.raw.clone());
        if saved.as_deref() != Some(raw) {
            self.violations.push("sent bytes that were not saved as pending first".into());
        }
        if self.die_on_send {
            self.die_on_send = false;
            return Err("the canister died here".into());
        }
        self.sent.push(raw.to_vec());
        let hash = evmtx::keccak(raw);
        if !self.receipts.contains_key(&hash) && !self.mempool.iter().any(|r| r == raw) {
            self.mempool.push(raw.to_vec());
        }
        Ok(())
    }
    async fn evm_seen(&mut self, hash: [u8; 32]) -> EvmView {
        if self.lag { return EvmView::NotYet; }
        match self.receipts.get(&hash) {
            Some(ok) => EvmView::Mined { success: *ok },
            None if self.mempool.iter().any(|r| evmtx::keccak(r) == hash) => EvmView::Pending,
            None => EvmView::NotYet,
        }
    }
    fn save(&mut self, p: &Purchase) {
        self.saved = Some(p.clone());
        self.saves += 1;
    }
    async fn btc_sats(&mut self) -> Result<u64, String> { Ok(self.btc_sats) }
    async fn btc_seen(&mut self, txid: [u8; 32]) -> BtcView {
        if self.btc_confirmed.contains(&txid) { BtcView::Confirmed } else if self.btc_known.contains(&txid) { BtcView::Mempool } else { BtcView::NotYet }
    }
    async fn btc_send(&mut self, raw: &[u8]) -> Result<(), String> {
        self.btc_sent.push(raw.to_vec());
        Ok(())
    }
    async fn delivered_units(&mut self, _txid: [u8; 32], _q: &Quote) -> Result<u128, String> {
        self.delivered.ok_or("no runes indexer".into())
    }
    async fn sign_taproot(&mut self, _h: [u8; 32]) -> Result<Vec<u8>, String> {
        self.taproot_signed += 1;
        Ok(vec![0x5a; 64])
    }
    async fn input_clean(&mut self, o: etch_autobuyer::custody::Outpoint) -> Result<(), String> {
        use etch_autobuyer::custody::{buyer_may_sign, Contents};
        self.ord_asked += 1;
        let c = |i: Vec<String>, r: Vec<String>| Some(Contents { inscriptions: i, runes: r, value: 100_000 });
        let contents = match self.ord_says {
            None => c(vec![], vec![]),
            Some("inscription") => c(vec![format!("{}i0", "ab".repeat(32))], vec![]),
            Some("runes") => c(vec![], vec!["DOG•GO•TO•THE•MOON".into()]),
            Some(_) => None, // ord does not know it
        };
        buyer_may_sign(&o, contents.as_ref())
    }
    fn venue(&self) -> Option<&dyn Venue> { self.venue.as_ref().map(|v| v as &dyn Venue) }
    fn bridge(&self) -> Option<&dyn Bridge> { self.bridge.as_ref().map(|b| b as &dyn Bridge) }
    fn bridge_allowlist(&self) -> Vec<[u8; 20]> { vec![BRIDGE] }
    async fn reference_floor_sats(&mut self, _w: u128) -> Result<u64, String> { Ok(95_000) }
    async fn reference_cap_sats(&mut self, _w: u128) -> Result<u64, String> { Ok(self.cap.unwrap_or(60_000)) }
    /// The vault's registry() and listingId(), and the registry's listing(0), as the chain answers them.
    async fn eth_read(&mut self, to: [u8; 20], data: &[u8]) -> Result<Vec<u8>, String> {
        if self.disagree { return Err("providers disagree on eth_call; nothing is believed".into()); }
        let word = |v: u128| [vec![0u8; 16], v.to_be_bytes().to_vec()].concat();
        match (to, &data[..4]) {
            (VAULT, s) if s == policy::REGISTRY => Ok([vec![0u8; 12], REGISTRY_ADDR.to_vec()].concat()),
            (VAULT, s) if s == policy::LISTING_ID => Ok(word(0)),
            (REGISTRY_ADDR, s) if s == policy::LISTING && data[4..] == word(0)[..] =>
                Ok(listing_ret(VAULT, self.listing_paused, self.listing_btc.as_deref().unwrap_or(WINNER_ADDR))),
            _ => Err(format!("no read for {}", hex::encode(data))),
        }
    }
    fn btc_hrp(&self) -> Result<&'static str, String> { Ok("tb") }
}

/// The registry's record of this buyer's listing: a rune listing whose btcVault is WINNER's signet address (the program is 32
/// bytes of 0x02; encoded by BIP-350's reference algorithm, which also reproduces the rehearsal's funded address).
const WINNER_ADDR: &str = "tb1pqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpq7vgt27";
const REGISTRY_ADDR: [u8; 20] = [0x5e; 20];
/// `registry.listing(0)`'s return, in the layout `cast abi-encode` gives the `Listing` struct (see `listing.rs`'s fixture): a rune
/// listing named "MOON", ref "840000:3", divisibility 0, whose vault and btcVault are given.
fn listing_ret(vault: [u8; 20], paused: bool, btc: &str) -> Vec<u8> {
    let word = |v: u128| [vec![0u8; 16], v.to_be_bytes().to_vec()].concat();
    let text = |s: &str| {
        let mut b = word(s.len() as u128);
        let mut padded = s.as_bytes().to_vec();
        padded.resize(s.len().div_ceil(32) * 32, 0);
        b.extend(padded);
        b
    };
    let (name, rref, addr) = (text("MOON"), text("840000:3"), text(btc));
    let head = 9 * 32;
    let mut b = word(0x20);
    for w in [word(1), word(paused as u128), word(1), word(0), [vec![0u8; 12], vault.to_vec()].concat(), word(0),
        word(head as u128), word((head + name.len()) as u128), word((head + name.len() + rref.len()) as u128)] {
        b.extend(w);
    }
    b.extend(name);
    b.extend(rref);
    b.extend(addr);
    b
}

fn order() -> Order {
    Order { rune: "840000:3".into(), units: 1_000, recipient_script: WINNER.to_vec(), cap_ref: "cap".into() }
}
fn fresh(id: u64) -> Purchase {
    Purchase { id, phase: Phase::Watching, amount_wei: ONE_ETH, order: Some(order()), pending_evm: None }
}
fn tick(w: &mut World, p: &mut Purchase) -> String {
    let line = block_on(looper::step(w, p, false));
    println!("  {line}");
    line
}
/// The canister dies and restarts: all it has is what was saved.
fn restart(w: &World) -> Purchase {
    w.saved.clone().expect("something was saved")
}

// ---- the tests -------------------------------------------------------------------------------------------------------------

#[test]
fn a_purchase_runs_end_to_end_with_nobody_calling_each_step() {
    let mut w = World::new();
    let mut p = fresh(0);
    tick(&mut w, &mut p); // announce: signed, saved, sent
    assert!(matches!(p.pending_evm, Some(ref pe) if pe.step == EvmStep::Announce));
    w.mine();
    tick(&mut w, &mut p); // mined: Announced, from the vault's own readyAt
    assert_eq!(p.phase, Phase::Announced { ready_at: w.chain_time + HOUR });
    tick(&mut w, &mut p); // the hour has not passed
    assert!(p.pending_evm.is_none() && w.sent.iter().all(|r| *r == w.sent[0]), "nothing new is sent before readyAt (the announce went twice: once, then again while it was pending)");
    w.now += HOUR + 1;
    w.chain_time += HOUR + 1;
    tick(&mut w, &mut p); // pull
    w.mine();
    tick(&mut w, &mut p);
    assert!(matches!(p.phase, Phase::Pulled { .. }) && w.balance == ONE_ETH);
    tick(&mut w, &mut p); // bridge
    w.mine();
    tick(&mut w, &mut p);
    assert!(matches!(p.phase, Phase::Bridging { expect_sats: 100_000, .. }) && w.balance == 0);
    tick(&mut w, &mut p); // BTC not there yet
    w.btc_sats = 100_000;
    tick(&mut w, &mut p); // funded, and in the same tick the quote is recorded
    assert!(matches!(p.phase, Phase::Quoted { .. }));
    tick(&mut w, &mut p); // PSBT verified and signed, bytes saved
    let Phase::Signed { txid, ref raw_tx, .. } = p.phase else { panic!("{:?}", p.phase) };
    assert_eq!(&raw_tx[4..6], &[0x00, 0x01], "a segwit transaction with its witness");
    // the Bitcoin module's own parser reads the loop's bytes as one signed transaction, and the two byte orders agree: the
    // loop carries display order, the Bitcoin module internal order, and `btc_seen` reverses between them
    let parsed = etch_autobuyer::btcsend::parse_signed(raw_tx).expect("a signed transaction");
    let mut internal = txid;
    internal.reverse();
    assert!(parsed.segwit && parsed.txid == internal && parsed.txid_hex() == hex::encode(txid));
    let raw_tx = raw_tx.clone();
    tick(&mut w, &mut p); // broadcast, Bitcoin does not show it yet
    assert!(matches!(p.phase, Phase::Signed { .. }) && w.btc_sent == vec![raw_tx.clone()]);
    w.btc_known.insert(txid);
    tick(&mut w, &mut p); // sent again, the SAME bytes; Bitcoin has it
    assert!(matches!(p.phase, Phase::Broadcast { .. }) && w.btc_sent.iter().all(|r| *r == raw_tx));
    w.btc_confirmed.insert(txid);
    w.delivered = Some(1_000);
    tick(&mut w, &mut p); // confirmed and delivered -> Settle
    w.mine();
    tick(&mut w, &mut p);
    assert_eq!(p.phase, Phase::Settled);
    assert_eq!((w.vault[0].status, w.vault[0].settled_quantity), (3, 1_000));
    assert!(w.violations.is_empty(), "{:?}", w.violations);
}

#[test]
fn dying_after_the_save_and_before_the_send_sends_the_same_bytes_on_restart() {
    let mut w = World::new();
    w.die_on_send = true;
    let mut p = fresh(0);
    tick(&mut w, &mut p);
    assert!(w.sent.is_empty() && w.mempool.is_empty(), "it died inside the send");
    let saved_raw = w.saved.as_ref().unwrap().pending_evm.as_ref().unwrap().raw.clone();
    let mut p = restart(&w);
    let signed_before = w.signed;
    tick(&mut w, &mut p);
    assert_eq!(w.sent, vec![saved_raw], "the saved bytes, not new ones");
    assert_eq!(w.signed, signed_before, "nothing re-signed");
    w.mine();
    tick(&mut w, &mut p);
    assert!(matches!(p.phase, Phase::Announced { .. }) && w.vault.len() == 1, "announced once");
    assert!(w.violations.is_empty());
}

#[test]
fn a_lost_reply_is_answered_by_the_chain_never_by_a_second_call() {
    // announce sent and mined, then the canister restarts from a state that never saw the reply (pending lost with it)
    let mut w = World::new();
    let mut p = fresh(0);
    tick(&mut w, &mut p);
    w.mine();
    let mut p = Purchase { pending_evm: None, ..restart(&w) };
    let signed = w.signed;
    tick(&mut w, &mut p);
    assert!(matches!(p.phase, Phase::Announced { .. }), "adopted from the vault: {:?}", p.phase);
    assert_eq!((w.signed, w.vault.len()), (signed, 1), "no second announcement");
    // the same for pull: the vault says Pulled, we believe it
    w.now += HOUR + 1;
    w.chain_time += HOUR + 1;
    tick(&mut w, &mut p);
    w.mine();
    let mut p = Purchase { pending_evm: None, ..restart(&w) };
    tick(&mut w, &mut p);
    assert!(matches!(p.phase, Phase::Pulled { .. }), "{:?}", p.phase);
    assert_eq!(w.vault[0].status, 2);
}

#[test]
fn providers_that_disagree_mean_wait_and_nothing_is_signed_or_saved() {
    let mut w = World::new();
    w.disagree = true;
    let mut p = fresh(0);
    let line = tick(&mut w, &mut p);
    assert!(line.contains("waits") && line.contains("disagree"));
    assert_eq!((w.signed, w.saves, p.phase.clone()), (0, 0, Phase::Watching));
}

#[test]
fn one_evm_transaction_in_flight_across_all_purchases() {
    let mut w = World::new();
    let mut p = fresh(0);
    let line = block_on(looper::step(&mut w, &mut p, true));
    assert!(line.contains("another EVM transaction of ours is out"));
    assert_eq!(w.signed, 0);
}

#[test]
fn a_mined_revert_stops_the_purchase_rather_than_signing_it_again() {
    let mut w = World::new();
    let mut p = fresh(0);
    tick(&mut w, &mut p);
    w.revert_next = true;
    w.mine();
    tick(&mut w, &mut p);
    assert_eq!(p.phase, Phase::Stuck { why: Stuck::Reverted });
    let signed = w.signed;
    tick(&mut w, &mut p);
    assert_eq!(w.signed, signed, "nothing further is signed");
}

#[test]
fn a_used_nonce_with_our_transaction_invisible_is_a_wait_not_a_guess() {
    let mut w = World::new();
    let mut p = fresh(0);
    tick(&mut w, &mut p);
    w.mine();
    w.lag = true; // mined, but the read-back does not show it
    let line = tick(&mut w, &mut p);
    assert!(line.contains("waiting rather than guessing") && p.pending_evm.is_some(), "{line}");
    w.lag = false;
    tick(&mut w, &mut p);
    assert!(matches!(p.phase, Phase::Announced { .. }) && p.pending_evm.is_none());
}

#[test]
fn with_no_venue_or_no_bridge_nothing_is_announced() {
    for (v, b) in [(false, true), (true, false), (false, false)] {
        let mut w = World::new();
        if !v { w.venue = None; }
        if !b { w.bridge = None; }
        let mut p = fresh(0);
        let line = tick(&mut w, &mut p);
        assert!(line.contains("announces nothing it could not finish"), "{line}");
        assert_eq!(w.signed, 0);
    }
}

#[test]
fn a_venue_that_vanishes_mid_purchase_stops_it() {
    let mut w = World::new();
    w.venue = None;
    let mut p = Purchase { phase: Phase::Funded, ..fresh(0) };
    w.vault.push(VaultPurchase { status: 2, ready_at: 0, ready_at_block: 0, deadline: w.chain_time + 99 * HOUR, late: false,
        max_eth: ONE_ETH, quantity: 1_000, settled_quantity: 0, returned: 0, cap_ref: "cap".into() });
    tick(&mut w, &mut p);
    assert_eq!(p.phase, Phase::Stuck { why: Stuck::Unconfigured });
}

#[test]
fn a_call_that_would_revert_now_is_not_signed() {
    // the canister's clock says the hour is up; the vault's clock does not yet
    let mut w = World::new();
    let mut p = fresh(0);
    tick(&mut w, &mut p);
    w.mine();
    tick(&mut w, &mut p);
    w.now += HOUR + 1;
    let signed = w.signed;
    let line = tick(&mut w, &mut p);
    assert!(line.contains("would not succeed now") && line.contains("TooEarly"), "{line}");
    assert_eq!(w.signed, signed);
}

#[test]
fn a_vault_purchase_that_is_not_ours_is_refused() {
    let mut w = World::new();
    w.vault.push(VaultPurchase { status: 1, ready_at: 0, ready_at_block: 0, deadline: 0, late: false, max_eth: 5 * ONE_ETH,
        quantity: 1_000, settled_quantity: 0, returned: 0, cap_ref: "someone".into() });
    let mut p = fresh(0);
    tick(&mut w, &mut p);
    assert_eq!(p.phase, Phase::Stuck { why: Stuck::Mismatch });
    // and announcing into an id that is not the vault's next is refused
    let mut w = World::new();
    let mut p = fresh(3);
    let line = tick(&mut w, &mut p);
    assert!(line.contains("next id is 0, not 3"), "{line}");
}

#[test]
fn past_the_deadline_with_no_bitcoin_the_money_goes_back() {
    let mut w = World::new();
    w.bridge = Some(bridge());
    let mut p = fresh(0);
    tick(&mut w, &mut p);
    w.mine();
    tick(&mut w, &mut p);
    w.now += HOUR + 1;
    w.chain_time += HOUR + 1;
    tick(&mut w, &mut p);
    w.mine();
    tick(&mut w, &mut p);
    let Phase::Pulled { deadline } = p.phase else { panic!() };
    assert_eq!(deadline, w.vault[0].deadline - 3600, "the vault's deadline, less the margin");
    w.now = deadline + 1;
    w.chain_time = deadline + 1;
    w.balance += ONE_ETH / 100; // gas money
    tick(&mut w, &mut p); // -> Returning, returnFunds signed
    assert_eq!(p.phase, Phase::Returning);
    w.mine();
    tick(&mut w, &mut p); // mined
    tick(&mut w, &mut p); // the vault says Returned
    assert_eq!(w.vault[0].returned, ONE_ETH);
    assert_eq!(p.phase, Phase::Returned);
}

#[test]
fn one_purchase_at_a_time() {
    assert!(looper::can_open(&[]).is_ok());
    assert!(looper::can_open(&[Purchase { phase: Phase::Settled, ..fresh(0) }]).is_ok());
    assert!(looper::can_open(&[fresh(0)]).unwrap_err().contains("one purchase at a time"));
    assert!(looper::can_open(&[Purchase { phase: Phase::Stuck { why: Stuck::Bridge }, ..fresh(0) }]).is_ok());
}

#[test]
fn a_quote_that_is_not_the_order_is_not_recorded() {
    let mut w = World::new();
    let mut q = quote();
    q.recipient_script = SELLER.to_vec();
    w.venue = Some(VenueFixture { quote: q, ..venue() });
    w.btc_sats = 100_000;
    let mut p = Purchase { phase: Phase::Funded, ..fresh(0) };
    let line = tick(&mut w, &mut p);
    assert!(line.contains("other than the order"), "{line}");
    assert_eq!(p.phase, Phase::Funded);
}


// ---- docs/v2/trust-points.md §1: the quote is the chain's, not the operator's ----------------------------------------------

/// A purchase driven to the tick that records its quote: pulled, bridged, and the BTC just arrived.
fn to_quote(w: &mut World) -> Purchase {
    let mut p = fresh(0);
    tick(w, &mut p);
    w.mine();
    tick(w, &mut p);
    w.now += HOUR + 1;
    w.chain_time += HOUR + 1;
    tick(w, &mut p);
    w.mine();
    tick(w, &mut p);
    tick(w, &mut p);
    w.mine();
    tick(w, &mut p);
    assert!(matches!(p.phase, Phase::Bridging { .. }), "{:?}", p.phase);
    w.btc_sats = 100_000;
    p
}

#[test]
fn a_quote_that_pays_anywhere_but_the_listings_address_is_never_recorded() {
    let mut w = World::new();
    let mut p = to_quote(&mut w);
    // the listing's address is not the one the venue quotes (the order was opened for WINNER; the chain now says otherwise)
    w.listing_btc = Some("tb1pa7h6sqckxnagsapfj4czg3v20yl96309dx3xj63y420a6q6jquws5cs0z7".into());
    let line = tick(&mut w, &mut p);
    assert!(line.contains("the quote pays") && line.contains("not recorded"), "{line}");
    assert!(!matches!(p.phase, Phase::Quoted { .. }), "no quote is recorded: {:?}", p.phase);
    w.listing_btc = None;
    tick(&mut w, &mut p);
    assert!(matches!(p.phase, Phase::Quoted { .. }), "with the chain's own address it is");
}

#[test]
fn a_quote_that_would_spend_more_than_the_released_eth_is_worth_is_never_recorded() {
    let mut w = World::new();
    let mut p = to_quote(&mut w);
    w.cap = Some(49_999); // the venue's quote may spend 50,000
    let line = tick(&mut w, &mut p);
    assert!(line.contains("worth at most 49999") && line.contains("not recorded"), "{line}");
    assert!(!matches!(p.phase, Phase::Quoted { .. }));
}

#[test]
fn a_listing_paused_after_the_quote_signs_nothing() {
    let mut w = World::new();
    let mut p = to_quote(&mut w);
    tick(&mut w, &mut p);
    assert!(matches!(p.phase, Phase::Quoted { .. }));
    w.listing_paused = true;
    let signed = w.signed;
    let line = tick(&mut w, &mut p);
    assert!(line.contains("paused") && line.contains("nothing signed"), "{line}");
    assert!(matches!(p.phase, Phase::Quoted { signing: None, .. }) && w.signed == signed, "not bound, not signed");
    w.listing_paused = false;
    tick(&mut w, &mut p);
    assert!(matches!(p.phase, Phase::Signed { .. }), "unpaused, it signs");
}

#[test]
fn the_fake_listing_is_encoded_as_foundry_encodes_it() {
    // the encoder above, fed listing.rs's fixture values, gives the `cast abi-encode` bytes' decode back
    let l = policy::ret_listing(&listing_ret(VAULT, false, WINNER_ADDR)).unwrap();
    assert_eq!((l.is_rune, l.paused, l.vault, l.reference.as_str(), l.btc_vault.as_str()), (true, false, VAULT, "840000:3", WINNER_ADDR));
    assert_eq!(hex::encode(policy::script_of(WINNER_ADDR, "tb").unwrap()), hex::encode(WINNER), "WINNER's address pays WINNER");
}

// ---- docs/v3/wrapper.md, "Custody on its own key": a purchase never signs away an inscription, a rune, or custody --------

/// Drive a fresh purchase to `Quoted`, as the end-to-end test does.
fn to_quoted(w: &mut World, p: &mut Purchase) {
    tick(w, p);
    w.mine();
    tick(w, p);
    w.now += HOUR + 1;
    w.chain_time += HOUR + 1;
    tick(w, p);
    w.mine();
    tick(w, p);
    tick(w, p);
    w.mine();
    tick(w, p);
    w.btc_sats = 100_000;
    tick(w, p);
    assert!(matches!(p.phase, Phase::Quoted { signing: None, .. }), "{:?}", p.phase);
}

#[test]
fn a_venue_psbt_spending_an_inscribed_or_runed_output_of_ours_is_refused_before_anything_is_signed_or_bound() {
    for (says, why) in [("inscription", "inscription"), ("runes", "runes"), ("unknown", "does not know")] {
        let mut w = World::new();
        let mut p = fresh(0);
        to_quoted(&mut w, &mut p);
        w.ord_says = Some(says);
        let line = tick(&mut w, &mut p);
        assert!(line.contains(why), "{says}: {line}");
        assert!(matches!(p.phase, Phase::Quoted { signing: None, .. }), "{says}: not bound, so an honest PSBT can still follow");
        assert_eq!((w.taproot_signed, w.btc_sent.len()), (0, 0), "{says}: nothing signed, nothing sent");
        assert!(w.ord_asked >= 1);
        // the same PSBT, once ord shows the output clean, is signed
        w.ord_says = None;
        tick(&mut w, &mut p);
        assert!(matches!(p.phase, Phase::Signed { .. }), "{says}: {:?}", p.phase);
        assert!(w.taproot_signed >= 1);
    }
}

#[test]
fn a_venue_psbt_listing_a_custody_output_as_ours_is_refused_because_it_is_not_ours() {
    // the custody address is another key: an input paying it is not an input paying OUR script, whatever the PSBT asks us to sign
    const CUSTODY: &[u8] = &[0x51, 0x20, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3];
    let mut w = World::new();
    let mut p = fresh(0);
    to_quoted(&mut w, &mut p);
    // the venue's PSBT with its one input's witness utxo (the last OUR in the bytes; the change output's comes first) naming
    // the custody script: a truthful PSBT asking us to sign a custody output
    let mut bytes = venue().psbt;
    let at = bytes.windows(OUR.len()).rposition(|x| x == OUR).expect("the input's witness utxo names OUR");
    bytes[at..at + OUR.len()].copy_from_slice(CUSTODY);
    w.venue = Some(VenueFixture { psbt: bytes, ..venue() });
    let line = tick(&mut w, &mut p);
    assert!(line.contains("ForeignInputToSign(0)"), "{line}");
    assert_eq!(w.taproot_signed, 0);
    assert!(!matches!(p.phase, Phase::Signed { .. }));
}
