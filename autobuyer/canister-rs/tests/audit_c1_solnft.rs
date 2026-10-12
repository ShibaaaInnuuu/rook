//! Audit C1 (docs/v9/audit-2026-10/c1-solnft-flow.md): the Solana NFT flow (`solnftflow`), proofs of concept against the fake chain
//! (`tests/desk_common`) with the real mainnet accounts of `tests/fixtures/solnft/` (the setup of `tests/solnft_flow.rs`). Each PoC
//! passed by exhibiting the bug it names; the fix round of 2026-10-12 inverted them into the `fixed_c1_0x_…` tests below (the comment
//! kept, a "FIXED (…)" line added), beside tests of the fixes that had no PoC.
mod desk_common;
mod fakerpc;

use std::cell::RefCell;
use std::collections::BTreeMap;

use base64::Engine;
use candid::CandidType;
use desk_common::*;
use etch_autobuyer::books::ledger::{Destination, MintItemArgs, MintTo, Stage, LATE_DELIVERY_WINDOW};
use etch_autobuyer::books::{self, BooksError, ListingKind, ONE};
use etch_autobuyer::chainlink::ledger::{Effects, OpKind, Scope, Spec};
use etch_autobuyer::custodycap::{CapPolicy, CollectionCap, Entry};
use etch_autobuyer::desk::state;
use etch_autobuyer::outflow::Flow;
use etch_autobuyer::solkey::KeyPath;
use etch_autobuyer::solnft::{self, Standard};
use etch_autobuyer::solnftbuy as nbuy;
use etch_autobuyer::solnftflow::{self as flow, buy, deliver, SolNftIo};
use etch_autobuyer::solrpc::Account;
use etch_autobuyer::soltx::{self, b58_encode, key, Instruction, Pubkey};
use serde::Deserialize;
use serde_json::{json, Value};

const MAD_LADS: &str = "J1S9H3QjnRtBbbuD4HjPV6RpRhwuk4zKbxsnCHuTgh9w";
const C: u16 = 1;
const WX1: Pubkey = [0xC1; 32];

thread_local! {
    static CAPS: RefCell<Option<CapPolicy>> = const { RefCell::new(None) };
    /// The canister's custody book as the other flows left it (audit C1-02).
    static BOOK: RefCell<Vec<Entry>> = const { RefCell::new(vec![]) };
    /// The outflow limits refuse every act (audit C1-02), and the acts recorded.
    static OUTFLOW_SHUT: RefCell<bool> = const { RefCell::new(false) };
    static RECORDED: RefCell<Vec<String>> = const { RefCell::new(vec![]) };
    /// Every simulation fails at the durable nonce's advance (message index 0): not the destination's (audit C1-07).
    static NONCE_FAULT: RefCell<bool> = const { RefCell::new(false) };
}

impl SolNftIo for World {
    fn now(&self) -> u64 {
        self.now
    }
    fn paused(&self) -> bool {
        false
    }
    fn caps(&self) -> Option<CapPolicy> {
        CAPS.with(|c| c.borrow().clone())
    }
    async fn lamports_per_eth(&mut self) -> Result<u64, String> {
        Ok(22_840_000_000)
    }
    fn cap_book(&self) -> etch_autobuyer::custodycap::Book {
        etch_autobuyer::custodycap::Book { entries: BOOK.with(|b| b.borrow().clone()) }
    }
    fn outflow_check(&mut self, key: &str, _: &str, _: Flow, _: u128) -> Result<(), String> {
        if OUTFLOW_SHUT.with(|o| *o.borrow()) {
            return Err(format!("waits (outflow limit) until later: {key}"));
        }
        Ok(())
    }
    fn outflow_record(&mut self, key: &str, _: &str, _: Flow, _: u128) {
        RECORDED.with(|r| r.borrow_mut().push(key.to_string()));
    }
}

// ---- the setup of tests/solnft_flow.rs --------------------------------------------------------------------------------------------------

fn m1() -> Pubkey {
    pk(KeyPath::Coll(C as u32))
}

fn bad(m: &str) -> Value {
    json!({ "InstructionError": [0, { "Custom": m }] })
}

fn token_record() -> Vec<u8> {
    vec![11, 255, 0, 0, 0, 0, 0]
}

fn frozen_token(mint: Pubkey, owner: Pubkey) -> Vec<u8> {
    let mut d = token_data(mint, owner, 1);
    d[108] = 2;
    d
}

/// Tensor `buy_legacy` (pNFT) as the program leaves the accounts (as `tests/solnft_flow.rs`).
fn tensor_buy(ix: &Instruction, acc: &mut BTreeMap<Pubkey, Account>) -> Result<(), Value> {
    let k = |i: usize| ix.accounts[i].key;
    let (buyer, buyer_ta, list_ta, list_state, mint, owner, payer) = (k(1), k(2), k(3), k(4), k(5), k(6), k(7));
    let ls = nbuy::decode_list_state(&acc.get(&list_state).ok_or_else(|| bad("Tensor: no listing"))?.data).map_err(|e| bad(&e))?;
    let max = u64::from_le_bytes(ix.data[8..16].try_into().unwrap());
    let spend = max + ls.amount * 200 / 10_000;
    let refund = acc.get(&solnft::token_record_address(&mint, &list_ta)).map(|a| a.lamports).unwrap_or(0);
    let p = acc.get_mut(&payer).ok_or_else(|| bad("Tensor: no payer"))?;
    p.lamports = (p.lamports + refund).checked_sub(spend + nbuy::ATA_RENT + nbuy::TOKEN_RECORD_RENT).ok_or_else(|| json!("InsufficientFundsForFee"))?;
    acc.entry(owner).or_insert(Account { lamports: 0, owner: soltx::SYSTEM_PROGRAM, executable: false, data: vec![] }).lamports += spend;
    for gone in [list_state, list_ta, solnft::token_record_address(&mint, &list_ta)] {
        acc.remove(&gone);
    }
    acc.insert(buyer_ta, Account { lamports: nbuy::ATA_RENT, owner: key(soltx::TOKEN_PROGRAM), executable: false, data: frozen_token(mint, buyer) });
    acc.insert(solnft::token_record_address(&mint, &buyer_ta), Account { lamports: nbuy::TOKEN_RECORD_RENT, owner: key(solnft::TOKEN_METADATA),
        executable: false, data: token_record() });
    Ok(())
}

/// Token Metadata `TransferV1` of a pNFT, wallet to wallet.
fn tm_transfer(ix: &Instruction, acc: &mut BTreeMap<Pubkey, Account>) -> Result<(), Value> {
    let k = |i: usize| ix.accounts[i].key;
    let (src, dst, to, mint) = (k(0), k(2), k(3), k(4));
    let s = acc.get(&src).ok_or_else(|| bad("TransferV1: no source"))?;
    if s.data[0..32] != mint || u64::from_le_bytes(s.data[64..72].try_into().unwrap()) != 1 {
        return Err(bad("TransferV1: the source does not hold it"));
    }
    acc.remove(&src);
    acc.remove(&solnft::token_record_address(&mint, &src));
    acc.insert(dst, Account { lamports: nbuy::ATA_RENT, owner: key(soltx::TOKEN_PROGRAM), executable: false, data: frozen_token(mint, to) });
    acc.insert(solnft::token_record_address(&mint, &dst), Account { lamports: nbuy::TOKEN_RECORD_RENT, owner: key(solnft::TOKEN_METADATA),
        executable: false, data: token_record() });
    Ok(())
}

/// The fake hands the closures the instructions after the durable nonce's advance; Solana names an error by the MESSAGE's index:
/// instruction `i` here is `i + 1` there.
fn at(i: usize, mut e: Value) -> Value {
    if let Some(x) = e.get_mut("InstructionError").and_then(|a| a.get_mut(0)) {
        *x = json!(i + 1);
    }
    e
}

fn exec(ixs: &[Instruction], acc: &mut BTreeMap<Pubkey, Account>) -> Result<(), Value> {
    if NONCE_FAULT.with(|f| *f.borrow()) {
        return Err(json!({ "InstructionError": [0, { "Custom": 6 }] }));
    }
    for (i, ix) in ixs.iter().enumerate() {
        if ix.program == key(nbuy::TENSOR_MARKETPLACE) {
            tensor_buy(ix, acc).map_err(|e| at(i, e))?;
        } else if ix.program == key(nbuy::M2) {
            m2_buy(ix, acc).map_err(|e| at(i, e))?;
        } else if ix.program == key(solnft::TOKEN_METADATA) && ix.data.first() == Some(&49) {
            tm_transfer(ix, acc).map_err(|e| at(i, e))?;
        } else {
            run_ixs(std::slice::from_ref(ix), acc).map_err(|e| at(i, e))?;
        }
    }
    Ok(())
}

fn world() -> World {
    let w = World::new();
    flow::reset();
    CAPS.with(|c| *c.borrow_mut() = None);
    BOOK.with(|b| b.borrow_mut().clear());
    OUTFLOW_SHUT.with(|o| *o.borrow_mut() = false);
    RECORDED.with(|r| r.borrow_mut().clear());
    NONCE_FAULT.with(|f| *f.borrow_mut() = false);
    {
        let mut c = w.chain.borrow_mut();
        c.set_account(m1(), soltx::SYSTEM_PROGRAM, RENT0, vec![]);
        c.set_account(WX1, key(soltx::TOKEN_PROGRAM), 1_461_600, mint_data(m1(), 0));
        c.exec = Some(Box::new(|ixs, _, acc, _| exec(ixs, acc)));
        c.sim = Some(Box::new(|ixs, _, acc, _| exec(ixs, &mut acc.clone())));
    }
    state::with_mut(|s| {
        s.books.add_collection(C, ListingKind::SolNft { collection: key(MAD_LADS), custodian: me() }, WX1, books::default_params(20 * SOL)).unwrap()
    });
    w
}

fn plant(w: &mut World, name: &str) -> Value {
    let p = format!("{}/tests/fixtures/solnft/{name}.json", env!("CARGO_MANIFEST_DIR"));
    let j: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    for x in j["accounts"].as_array().unwrap() {
        let data = base64::engine::general_purpose::STANDARD.decode(x["data"].as_str().unwrap()).unwrap();
        w.plant(soltx::pubkey(x["pubkey"].as_str().unwrap()).unwrap(), soltx::pubkey(x["owner"].as_str().unwrap()).unwrap(),
            x["lamports"].as_u64().unwrap(), data);
    }
    j["facts"].clone()
}

fn fact(f: &Value, k: &str) -> Pubkey {
    soltx::pubkey(f[k].as_str().unwrap()).unwrap()
}

fn fund(w: &mut World, floor: u64) {
    let now = w.now as i64;
    let lamports = 3 * floor;
    state::with_mut(|s| {
        s.books.post_floor(C, floor, 9_000, 20, now).unwrap();
        s.books.mint(C, lamports, 0, now).unwrap();
    });
    let supply = state::with(|s| s.books.get(C).unwrap().ledger.sol_backed);
    w.plant(WX1, key(soltx::TOKEN_PROGRAM), 1_461_600, mint_data(m1(), supply));
    w.deposit_sol(&m1(), lamports);
}

fn tensor_evidence(f: &Value) -> String {
    json!([{ "listingSource": "TENSOR_MARKETPLACE_LISTING", "pdaAddress": f["list_state"], "tokenMint": f["asset"], "price": 10.0 }]).to_string()
}

fn stats(floor: u64) -> String {
    json!({ "floorPrice": floor }).to_string()
}

fn tick(w: &mut World) -> Vec<String> {
    let now = w.now;
    let l = block_on(flow::tick(w, now));
    for x in &l {
        println!("{x}");
    }
    l
}

fn coll() -> books::Collection {
    state::with(|s| s.books.get(C).unwrap().clone())
}

/// A Tensor purchase of Mad Lads set up and signed: the asset.
fn bought(w: &mut World) -> Pubkey {
    let f = plant(w, "tensor_pnft_mad_lads");
    fund(w, 10_700_000_000);
    flow::floor::put_evidence(C, &tensor_evidence(&f), &stats(10_700_000_000), w.now).unwrap();
    let l = tick(w);
    assert!(l.iter().any(|x| x.contains("advance + purchase") && x.contains("signed and sent")), "{l:?}");
    fact(&f, "asset")
}

/// [`bought`], landed and booked: the item held by the books.
fn held(w: &mut World) -> Pubkey {
    let a = bought(w);
    w.settle();
    tick(w);
    assert!(in_custody(w, &a, &me()) && coll().ledger.held == 1);
    a
}

fn in_custody(w: &World, asset: &Pubkey, holder: &Pubkey) -> bool {
    let a: solnft::Accounts = solnft::check_keys(asset, &key(MAD_LADS), Standard::Programmable, holder).into_iter()
        .map(|k| (k, w.chain.borrow().accounts.get(&k).cloned())).collect();
    solnft::check_on(&a, asset, &key(MAD_LADS), Standard::Programmable, holder).is_ok()
}

fn redeem_and_resolve(w: &mut World, owner: u8, d: Destination) -> u64 {
    let now = w.now as i64;
    state::with_mut(|s| {
        let t = s.books.redeem(C, wallet(owner), d, now).unwrap();
        s.books.collections.get_mut(&C).unwrap().ledger.tickets.get_mut(&t).unwrap().round = books::math::round_at(now);
        s.books.resolve(C, t, Some([5; 32]), now).unwrap();
        t
    })
}

/// The item `asset` moved from `from`'s token account to `to`'s (a transfer outside the canister).
fn move_item(w: &mut World, asset: &Pubkey, from: &Pubkey, to: &Pubkey) {
    let mut c = w.chain.borrow_mut();
    let ata = soltx::associated_token_address(from, asset);
    c.accounts.remove(&solnft::token_record_address(asset, &ata));
    c.accounts.remove(&ata);
    let ata2 = soltx::associated_token_address(to, asset);
    c.set_account(ata2, key(soltx::TOKEN_PROGRAM), nbuy::ATA_RENT, frozen_token(*asset, *to));
    c.set_account(solnft::token_record_address(asset, &ata2), key(solnft::TOKEN_METADATA), nbuy::TOKEN_RECORD_RENT, token_record());
}

/// An item of the collection minted into the books for a holder (`wallet(owner)`), with its key: `B` (a real Mad Lads mint, in
/// `K_main`'s custody) when `on_chain`, else a key nothing holds.
fn mint_for(w: &mut World, k: Pubkey, on_chain: bool, owner: u8) {
    if on_chain {
        let ata = soltx::associated_token_address(&me(), &k);
        w.plant(ata, key(soltx::TOKEN_PROGRAM), nbuy::ATA_RENT, frozen_token(k, me()));
        w.plant(solnft::token_record_address(&k, &ata), key(solnft::TOKEN_METADATA), nbuy::TOKEN_RECORD_RENT, token_record());
    }
    let now = w.now as i64;
    state::with_mut(|s| {
        let args = MintItemArgs { key: k, source: books::math::keccak(&[&k, &[3u8; 32]]), tx: [3; 32], vout: 0, deposit: false,
            inscription_id: String::new(), proof: vec![] };
        s.books.mint_item(C, args, MintTo::Holder(wallet(owner)), now).unwrap();
    });
}

// ---- C1-01 (High) -----------------------------------------------------------------------------------------------------------------------

/// C1-01. A delivery that LANDED is booked `record_delivery` only if the destination still holds the item at the canister's later read;
/// otherwise `note_sent`, and the ticket stays `Resolved` with `sent_tx` set, re-checked every pass for ever. `deliver::next_ticket` takes
/// the FIRST resolved ticket of the collection, and `deliver::pass` returns `None` when its re-check fails: every later ticket of the
/// collection is never delivered (its redeemer gets a refund 37 days later, never the item), and the gone item stays `held` (its
/// marker, the custody cap's count) for ever. The redeemer of the first ticket only has to move the item on (a transfer, a Tensor
/// listing, a staking delegate) before the canister reads it: one redemption shuts the collection's deliveries for good.
///
/// A fix must refuse this: a landed, successful canonical transfer (or `K_main` no longer holding the item at finalized after it) is
/// `record_delivery`, whatever the destination did since; and one ticket that cannot progress never stops the others.
///
/// FIXED (2026-10-12): `deliver::landed` books `record_delivery` on the line's verdict alone (no read of the destination), and
/// `deliver::pass` walks every actionable ticket: one whose item is not in `K_main`'s custody is skipped, the next is delivered.
#[test]
fn fixed_c1_01_one_redeemer_moving_the_item_on_no_longer_stops_any_delivery_of_the_collection() {
    let mut w = world();
    let a = held(&mut w);

    // the attacker redeems one unit: ticket 0, to their wallet, draws A
    let to0 = pk(KeyPath::Intent(77));
    let t0 = redeem_and_resolve(&mut w, 1, Destination::Sol(to0));
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("ticket 0") && x.contains("signed")), "{l:?}");
    w.settle();
    assert!(in_custody(&w, &a, &to0), "the delivery landed: the attacker holds A");
    // ... and moves it on (to another wallet) before the canister's read of the destination
    move_item(&mut w, &a, &to0, &wallet(9));
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("ticket 0's delivery landed") && x.contains("recorded delivered")), "{l:?}");
    assert!(l.iter().all(|x| !x.contains("noted sent")), "{l:?}");
    let c = coll();
    assert_eq!(c.ledger.tickets[&t0].stage, Stage::Delivered, "the landing decided it");
    assert_eq!(c.ledger.held, 0);
    assert!(!c.ledger.held_markers.contains_key(&a), "the gone item no longer counts as held");

    // two more items: X (in the books, but NOT in K_main's custody) and B (a real Mad Lads mint in K_main's custody); ticket 1 draws
    // X, ticket 2 draws B
    let fb = plant(&mut w, "m2_mad_lads_4vCV2b");
    let b = fact(&fb, "asset");
    let x = [0x77; 32];
    mint_for(&mut w, x, false, 2);
    mint_for(&mut w, b, true, 2);
    assert!(in_custody(&w, &b, &me()), "B is deliverable");
    let (to1, to2) = (pk(KeyPath::Intent(78)), pk(KeyPath::Intent(79)));
    let t1 = redeem_and_resolve(&mut w, 2, Destination::Sol(to1));
    let t2 = redeem_and_resolve(&mut w, 2, Destination::Sol(to2));
    let item = |t: u64| coll().ledger.items[&coll().ledger.tickets[&t].item_id].key;
    if item(t1) != x {
        state::with_mut(|s| {
            let tk = &mut s.books.collections.get_mut(&C).unwrap().ledger.tickets;
            let (i1, i2) = (tk[&t1].item_id, tk[&t2].item_id);
            tk.get_mut(&t1).unwrap().item_id = i2;
            tk.get_mut(&t2).unwrap().item_id = i1;
        });
    }
    assert_eq!((item(t1), item(t2)), (x, b));

    // ticket 1 cannot progress (its item is not in custody): it is skipped, ticket 2 is delivered
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains(&format!("ticket {t1}")) && x.contains("not in K_main's custody")), "{l:?}");
    assert!(l.iter().any(|x| x.contains(&format!("ticket {t2} to")) && x.contains("signed")), "{l:?}");
    w.settle();
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains(&format!("ticket {t2}'s delivery landed")) && x.contains("recorded delivered")), "{l:?}");
    assert!(in_custody(&w, &b, &to2) && !in_custody(&w, &b, &me()), "B reached the honest redeemer");
    let c = coll();
    assert_eq!(c.ledger.tickets[&t2].stage, Stage::Delivered);
    assert_eq!(c.ledger.tickets[&t1].stage, Stage::Resolved, "ticket 1 waits (its refund rules apply)");
    assert_eq!(c.ledger.held, 1, "only X, which the books hold and custody does not");
}

/// C1-01, a ticket an older build left noted sent (its delivery landed, the destination's read failed): recorded at the next pass,
/// whatever the destination holds now.
#[test]
fn fixed_c1_01_a_ticket_noted_sent_by_an_older_build_is_recorded_delivered() {
    let mut w = world();
    let a = held(&mut w);
    let to = pk(KeyPath::Intent(77));
    let t = redeem_and_resolve(&mut w, 1, Destination::Sol(to));
    // the older build: the delivery landed, the destination moved it on, `note_sent`
    move_item(&mut w, &a, &me(), &wallet(9));
    state::with_mut(|s| s.books.note_sent(C, t, [8; 32]).unwrap());
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains(&format!("ticket {t} recorded delivered"))), "{l:?}");
    let c = coll();
    assert_eq!((c.ledger.tickets[&t].stage, c.ledger.tickets[&t].delivery_tx, c.ledger.held), (Stage::Delivered, [8; 32], 0));
}

// ---- C1-02 (Medium) ---------------------------------------------------------------------------------------------------------------------

/// C1-02. The install's caps (`canister::caps_of`) name no Solana NFT collection (stage 0 lists the wrappers, the Inscriptions
/// listings and the Ethereum collections), so `buy::plan_and_sign` falls back to `custodycap::stage0(&[sol:<collection>])`: a policy of
/// THIS collection alone, whose global cap is sized from it, checked against a book of THIS collection's items alone. The canister-wide
/// global item and value caps (and A4-L-1's exit-room bound on them) never see a Solana NFT, and no other flow's cap counts them.
///
/// A fix must refuse this buy: a Solana NFT listing is a collection of the install's policy (stage 0 included), and the global caps
/// are checked against every collection's holdings.
///
/// FIXED (2026-10-12): the flow checks the install's policy as it is (`caps_of` lists every SolNft listing at stage 0: an unnamed
/// collection is `NotListed`) against ONE book: the canister's (`cap_book`) plus every SolNft listing's items (`cap_entries`, also
/// added to every other flow's `with_caps`); deliveries ask and record the outflow limits.
#[test]
fn fixed_c1_02_a_full_global_custody_cap_stops_a_solana_nft_buy() {
    let mut w = world();
    let f = plant(&mut w, "tensor_pnft_mad_lads");
    fund(&mut w, 10_700_000_000);
    // the install's policy: one Bitcoin wrapper, and a global cap of NO item at all (any acquisition under it is refused)
    let btc = "btc:0x00000000000000000000000000000000000000cc";
    let p = CapPolicy { collections: vec![CollectionCap { collection: btc.into(), max_items: 3,
        max_value_wei: 10u128.pow(19), max_item_wei: 10u128.pow(19) }], global_max_items: 0, global_max_value_wei: 0 };
    assert!(etch_autobuyer::custodycap::check_acquire(&p, &Default::default(), btc, 1).is_err());
    CAPS.with(|c| *c.borrow_mut() = Some(p));
    flow::floor::put_evidence(C, &tensor_evidence(&f), &stats(10_700_000_000), w.now).unwrap();
    let signed = w.signer.signed();
    let l = tick(&mut w);
    assert!(l.iter().all(|x| !x.contains("signed and sent")), "the buy is refused: {l:?}");
    assert!(flow::api::view(C).unwrap().note.contains("NotListed"), "{}", flow::api::view(C).unwrap().note);
    assert_eq!((w.signer.signed(), coll().module.transit_units), (signed, 0));

    // the collection named, but the canister's book (another flow's two items) fills the global item cap
    let k = flow::cap_key(&key(MAD_LADS));
    CAPS.with(|c| *c.borrow_mut() = Some(CapPolicy { collections: vec![CollectionCap { collection: k.clone(), max_items: 3,
        max_value_wei: 10u128.pow(19), max_item_wei: 10u128.pow(19) }, CollectionCap { collection: btc.into(), max_items: 2,
        max_value_wei: 10u128.pow(19), max_item_wei: 10u128.pow(19) }], global_max_items: 2, global_max_value_wei: 10u128.pow(19) }));
    BOOK.with(|b| *b.borrow_mut() = (0..2).map(|i| Entry { id: format!("i{i}"), collection: btc.into(), value_wei: 1, in_flight: false }).collect());
    w.advance_time(10);
    flow::floor::put_evidence(C, &tensor_evidence(&f), &stats(10_700_000_000), w.now).unwrap();
    let l = tick(&mut w);
    assert!(l.iter().all(|x| !x.contains("signed and sent")), "{l:?}");
    assert!(flow::api::view(C).unwrap().note.contains("GlobalItems"), "{}", flow::api::view(C).unwrap().note);

    // room again: bought; the item then counts in the canister-wide book every flow checks
    BOOK.with(|b| b.borrow_mut().clear());
    w.advance_time(10);
    flow::floor::put_evidence(C, &tensor_evidence(&f), &stats(10_700_000_000), w.now).unwrap();
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("signed and sent")), "{l:?}");
}

/// C1-02: the item a buy brought counts in the canister-wide book (in flight once landed, held once booked), and the default policy
/// names every Solana NFT listing.
#[test]
fn fixed_c1_02_the_flows_items_are_in_the_canister_wide_book_and_the_default_policy() {
    let mut w = world();
    assert_eq!(flow::live_cap_keys(), vec![flow::cap_key(&key(MAD_LADS))]);
    let a = bought(&mut w);
    assert!(flow::cap_entries(None).is_empty(), "signed, not landed: the advance's own record bounds it");
    w.settle();
    let now = w.now;
    block_on(flow::run::settle(&mut w, now));
    let e = flow::cap_entries(None);
    assert_eq!(e.len(), 1);
    assert!(e[0].in_flight && e[0].id == format!("{}{}", flow::CAP_ID_PREFIX, soltx::b58_encode(&a)) && e[0].value_wei > 0, "{e:?}");
    tick(&mut w);
    let e = flow::cap_entries(None);
    assert!(e.len() == 1 && !e[0].in_flight && e[0].collection == flow::cap_key(&key(MAD_LADS)), "{e:?}");
}

/// C1-02: a delivery asks the outflow limits before it is signed, and is counted once signed.
#[test]
fn fixed_c1_02_a_delivery_waits_on_the_outflow_limits_and_is_recorded() {
    let mut w = world();
    held(&mut w);
    let t = redeem_and_resolve(&mut w, 1, Destination::Sol(pk(KeyPath::Intent(77))));
    OUTFLOW_SHUT.with(|o| *o.borrow_mut() = true);
    let signed = w.signer.signed();
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("outflow limit")), "{l:?}");
    assert_eq!(w.signer.signed(), signed, "nothing signed");
    assert!(RECORDED.with(|r| r.borrow().is_empty()));
    OUTFLOW_SHUT.with(|o| *o.borrow_mut() = false);
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains(&format!("ticket {t} to")) && x.contains("signed")), "{l:?}");
    assert_eq!(RECORDED.with(|r| r.borrow().clone()), vec![format!("solnft:{C}:deliver:{t}")]);
}

// ---- C1-03 (Medium) ---------------------------------------------------------------------------------------------------------------------

/// C1-03. With M2 switched on (`sol_nft_configure(.., m2: Some(true))`), the cheapest ask of a pNFT collection is typically an M2
/// listing. Its purchase (`solnftm2::buy_instructions`, 1,135–1,201 bytes alone) composed with the advance (a second signer), the
/// link's memo and the nonce advance is 1,376–1,442 bytes: over Solana's 1,232. Every pass picks it again (`buy::pick` keeps no
/// memory of a refusal), books the advance, fails at the size check and undoes it: nothing is bought, and the cheapest keyless Tensor
/// ask under the same limit, which the flow buys with M2 off, never is.
///
/// A fix must refuse to PICK what cannot be built (compile the whole transaction before choosing, or a v0 message with a lookup
/// table), and fall through to the next ask when one is refused.
///
/// FIXED (2026-10-12): `plan_and_sign` builds, judges and sizes each candidate as it will be signed (`buy::composed_size`) BEFORE
/// anything is booked; one that does not fit is skipped (remembered for the rest of the read) and the next ask is bought.
#[test]
fn fixed_c1_03_m2_switched_on_no_longer_blocks_the_buy_of_a_pnft_collection() {
    let mut w = world();
    let ft = plant(&mut w, "tensor_pnft_mad_lads");
    let fm = plant(&mut w, "m2_mad_lads_4vCV2b");
    fund(&mut w, 10_700_000_000);
    flow::with_mut(|s| s.cols.entry(C).or_default().m2 = true);
    let ev = json!([
        { "listingSource": "TENSOR_MARKETPLACE_LISTING", "pdaAddress": ft["list_state"], "tokenMint": ft["asset"], "price": 10.2 },
        { "listingSource": "M2", "pdaAddress": fm["seller_trade_state"], "tokenMint": fm["asset"], "price": 8.352 },
    ]).to_string();
    flow::floor::put_evidence(C, &ev, &stats(10_700_000_000), w.now).unwrap();
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("2 asks verified")), "both asks verified: {l:?}");
    let v = flow::api::view(C).unwrap();
    assert_eq!(v.asks.len(), 1, "the one bought from is dropped from the read");
    assert!(v.asks[0].venue == nbuy::Venue::M2 && v.asks[0].total < 10 * SOL, "the M2 ask, the cheaper, is left");
    assert!(l.iter().any(|x| x.contains("on Tensor") && x.contains("signed and sent")), "the Tensor ask is bought at once: {l:?}");
    assert!(l.iter().all(|x| !x.contains("Solana's limit is 1232")), "nothing too large is ever booked: {l:?}");
    assert_eq!(state::with(|s| s.link.links.len()), 1, "one advance, the Tensor purchase's");
    // FIXED FURTHER (2026-10-12): with no lookup table yet the M2 ask WAITS for it (an M2 purchase is a v0 message loading M2's fixed
    // accounts from the canister's table, `solnftflow::table`): skipped this pass, not remembered as unbuyable, never booked
    assert!(flow::cache(C, |c| c.refused.is_empty()), "the M2 ask is not refused: it waits for the lookup table");
    assert!(flow::with(|s| s.table.is_none()), "the table is made on a pass that signs nothing else");
}

/// C1-03, the measure: every M2 pNFT fixture's purchase fits alone but not as the flow sends it (nonce advance, the advance transfer
/// with its second signer, the memo); the legacy one fits. A fix must make every pNFT line here fit (or never pick such an ask).
///
/// FIXED (2026-10-12): `buy::composed_size` measures exactly that composition (with the longest memo a link can carry); it is over the
/// packet for every M2 pNFT fixture (so such an ask is never picked) and within it for the legacy M2 and the Tensor purchase.
///
/// FIXED FURTHER (2026-10-12): the flow signs an M2 purchase as a v0 message loading M2's fixed accounts from the canister's lookup
/// table (`solnftflow::table`): every M2 fixture then fits with 128 bytes to spare at least; the keyless purchases stay legacy messages
/// (Tensor's fit with 64 to spare at least; an MMM pNFT's does not and is skipped, a recorded residual).
#[test]
fn fixed_c1_03_the_composed_size_is_measured_before_an_ask_is_picked() {
    for name in ["m2_mad_lads_4vCV2b", "m2_degods_5eYaTG", "m2_okay_bears_3YEZJh", "m2_smb_gen3_4tBzuW", "m2_cyber_frogs_4ZmLtG",
        "m2_claynosaurz_AKchXM", "m2_famous_fox_federation_H811WY"] {
        let mut w = World::new();
        let f = plant(&mut w, name);
        let a: solnft::Accounts = w.chain.borrow().accounts.iter().map(|(k, v)| (*k, Some(v.clone()))).collect();
        let asset = fact(&f, "asset");
        let collection = fact(&f, "collection");
        let m = solnft::is_member(&asset, &collection, Standard::Programmable, &a).unwrap();
        let c = nbuy::Candidate { venue: nbuy::Venue::M2, listing: fact(&f, "seller_trade_state"), asset };
        let ask = nbuy::verify_ask(&c, &m, &a, w.now as i64).unwrap();
        let ixs = etch_autobuyer::solnftm2::buy_instructions(&me(), &ask, &m).unwrap();
        let alone = soltx::wire_size(&soltx::compile_parts(&ixs, me(), [1; 32]).unwrap().message());
        let composed = buy::composed_size(&me(), &m1(), C, &ixs, ask.total, &[]).unwrap();
        // FIXED FURTHER (2026-10-12): the same transaction as a v0 message with the canister's lookup table, as the flow signs it
        let nbuy::AskDetail::M2 { house: Some(h), .. } = &ask.detail else { panic!("{name}: no auction house") };
        let t = soltx::LookupTable { key: [0x7A; 32], addresses: flow::table::fixed_addresses(&me(), h) };
        let v0 = buy::composed_size(&me(), &m1(), C, &ixs, ask.total, &[t]).unwrap();
        println!("{name}: {:?}, {} creators: alone {alone}, composed {composed} (legacy), {v0} (v0 with the lookup table)", m.standard,
            m.creators.len());
        assert!(alone <= soltx::PACKET_DATA_SIZE, "{name}");
        if m.standard == Standard::Programmable {
            assert!(soltx::check_size(composed).is_err(), "{name}: {composed}: never picked as a legacy message");
        } else {
            assert!(soltx::check_size(composed).is_ok(), "{name}: {composed}");
        }
        assert!(v0 + 128 <= soltx::PACKET_DATA_SIZE, "{name}: {v0}: fits with a margin of 128 bytes at least");
    }
    // the keyless venues (Tensor, MMM), legacy messages as before: every standard fits
    for (name, venue, listing, std) in [("tensor_pnft_mad_lads", nbuy::Venue::Tensor, "list_state", Standard::Programmable),
        ("tensor_legacy_fff", nbuy::Venue::Tensor, "list_state", Standard::Legacy), ("tensor_core_otc", nbuy::Venue::Tensor, "list_state", Standard::Core),
        ("mmm_pnft_smb_gen3", nbuy::Venue::Mmm, "sell_state", Standard::Programmable), ("mmm_legacy_tff", nbuy::Venue::Mmm, "sell_state", Standard::Legacy)] {
        let mut w = World::new();
        let f = plant(&mut w, name);
        let a: solnft::Accounts = w.chain.borrow().accounts.iter().map(|(k, v)| (*k, Some(v.clone()))).collect();
        let asset = fact(&f, "asset");
        let m = solnft::is_member(&asset, &fact(&f, "collection"), std, &a).unwrap();
        let ask = nbuy::verify_ask(&nbuy::Candidate { venue, listing: fact(&f, listing), asset }, &m, &a, w.now as i64).unwrap();
        let ixs = buy::purchase(&me(), &ask, &m).unwrap();
        let n = buy::composed_size(&me(), &m1(), C, &ixs, ask.total, &[]).unwrap();
        println!("{name}: composed {n} (legacy)");
        if name == "mmm_pnft_smb_gen3" {
            // a residual of C1-03 (recorded 2026-10-12): an MMM pNFT purchase is over the packet as a legacy message, so it is skipped
            // before anything is booked (the lookup table serves M2 purchases only)
            assert!(soltx::check_size(n).is_err(), "{name}: {n}");
        } else {
            assert!(n + 64 <= soltx::PACKET_DATA_SIZE, "{name}: {n}");
        }
    }
}

// ---- C1-03 fixed further (2026-10-12): Magic Eden M2 on by default, a pNFT purchase as a v0 message with the lookup table ------------

/// M2's purchase as the program leaves the accounts (LiteSVM runs the real program: `autobuyer/solnft-svm/tests/m2_table.rs`):
/// `deposit` moves the all-in into the buyer's escrow, `buy_v2` writes nothing here, `mip1_execute_sale_v2` pays the seller out of the
/// escrow (the rest to the treasury: fee and royalty), closes the seller's trade state, token account and token record, and gives the
/// buyer the item (a frozen token account and an unlocked token record), the buyer paying their rent.
fn m2_buy(ix: &Instruction, acc: &mut BTreeMap<Pubkey, Account>) -> Result<(), Value> {
    let k = |i: usize| ix.accounts[i].key;
    let disc = |n: &str| ix.data[..8] == soltx::anchor_disc("global", n);
    if disc("deposit") {
        let (payer, escrow) = (k(0), k(2));
        let amount = u64::from_le_bytes(ix.data[9..17].try_into().unwrap());
        let p = acc.get_mut(&payer).ok_or_else(|| bad("M2: no payer"))?;
        p.lamports = p.lamports.checked_sub(amount).ok_or_else(|| json!("InsufficientFundsForFee"))?;
        acc.entry(escrow).or_insert(Account { lamports: 0, owner: key(nbuy::M2), executable: false, data: vec![] }).lamports += amount;
    } else if disc("mip1_execute_sale_v2") {
        let (payer, seller, token_account, buyer_ta, mint, treasury, sts, escrow) = (k(0), k(2), k(5), k(6), k(7), k(10), k(11), k(13));
        let price = u64::from_le_bytes(ix.data[8..16].try_into().unwrap());
        acc.remove(&sts).ok_or_else(|| bad("M2: no seller trade state"))?;
        let t = acc.get(&token_account).ok_or_else(|| bad("M2: no token account"))?;
        if t.data[0..32] != mint || u64::from_le_bytes(t.data[64..72].try_into().unwrap()) != 1 {
            return Err(bad("M2: the seller does not hold it"));
        }
        let e = acc.remove(&escrow).ok_or_else(|| bad("M2: no escrow"))?;
        let rest = e.lamports.checked_sub(price).ok_or_else(|| bad("M2: the escrow is short"))?;
        acc.entry(seller).or_insert(Account { lamports: 0, owner: soltx::SYSTEM_PROGRAM, executable: false, data: vec![] }).lamports += price;
        acc.entry(treasury).or_insert(Account { lamports: 0, owner: soltx::SYSTEM_PROGRAM, executable: false, data: vec![] }).lamports += rest;
        for gone in [token_account, solnft::token_record_address(&mint, &token_account)] {
            acc.remove(&gone);
        }
        let p = acc.get_mut(&payer).ok_or_else(|| bad("M2: no payer"))?;
        p.lamports = p.lamports.checked_sub(nbuy::ATA_RENT + nbuy::TOKEN_RECORD_RENT).ok_or_else(|| json!("InsufficientFundsForFee"))?;
        acc.insert(buyer_ta, Account { lamports: nbuy::ATA_RENT, owner: key(soltx::TOKEN_PROGRAM), executable: false, data: frozen_token(mint, payer) });
        acc.insert(solnft::token_record_address(&mint, &buyer_ta), Account { lamports: nbuy::TOKEN_RECORD_RENT, owner: key(solnft::TOKEN_METADATA),
            executable: false, data: token_record() });
    } else if !disc("buy_v2") {
        return Err(bad("M2: an unknown instruction"));
    }
    Ok(())
}

fn m2_evidence(f: &Value) -> String {
    json!([{ "listingSource": "M2", "pdaAddress": f["seller_trade_state"], "tokenMint": f["asset"], "price": 8.352 }]).to_string()
}

/// Mad Lads' M2 ask handed in, the canister's lookup table made and landed, the M2 purchase signed: the asset, and the module and
/// `M[1]`'s lamports before the advance.
fn bought_on_m2(w: &mut World) -> (Pubkey, etch_autobuyer::books::floor::Module, u64) {
    let f = plant(w, "m2_mad_lads_4vCV2b");
    fund(w, 10_700_000_000);
    flow::floor::put_evidence(C, &m2_evidence(&f), &stats(10_700_000_000), w.now).unwrap();
    // the first pass: the M2 ask waits for the lookup table (nothing booked), and the table is made (the one step of the pass)
    let l = tick(w);
    assert!(l.iter().any(|x| x.contains("the M2 lookup table") && x.contains("signed and sent")), "{l:?}");
    assert!(state::with(|s| s.link.links.is_empty()), "no advance booked while the table is not there");
    assert_eq!(coll().module.transit_units, 0);
    assert!(flow::cache(C, |c| c.refused.is_empty()), "the ask is not refused: it waits");
    let t = flow::with(|s| s.table.clone()).unwrap();
    assert!(!t.landed);
    let (module, m1_before) = (coll().module, w.lamports(&m1()));
    w.settle();
    // the second: the table landed, read at finalized (K_main's, active, what it was made with), and the M2 purchase signed as a v0 message
    let l = tick(w);
    assert!(l.iter().any(|x| x.contains("the lookup table landed")), "{l:?}");
    assert!(l.iter().any(|x| x.contains("on M2") && x.contains("signed and sent")), "{l:?}");
    let tx = w.chain.borrow().sent.last().unwrap().clone();
    let (sigs, msg) = soltx::split_tx(&tx).unwrap();
    let p = soltx::Parts::parse(msg).unwrap();
    assert_eq!((p.version, p.tables.len(), p.tables[0].table, sigs.len()), (Some(0), 1, t.key, 2), "v0, the canister's table, K_main and K_coll");
    println!("the Mad Lads M2 purchase as signed: {} bytes", tx.len());
    assert!(tx.len() + 128 <= soltx::PACKET_DATA_SIZE, "{} bytes", tx.len());
    assert_eq!(coll().module.transit_units, ONE, "the advance is booked, in the same transaction as the purchase");
    (fact(&f, "asset"), module, m1_before)
}

/// The operator's decision of 2026-10-12: M2 is bought buyer-only by default. Mad Lads' cheap asks are M2 listings of pNFTs, whose
/// purchase did not fit a legacy message (C1-03): it is now bought end to end from M2 in the fake chain. The advance `M[i] → K_main`
/// and the purchase are still ONE transaction, judged whole, signed by `K_main` and `K_coll[i]`, as a v0 message loading M2's fixed
/// accounts from the canister's lookup table; then the custody at finalized and `mint_item` + `settle_advance` (B1-06's writer).
#[test]
fn fixed_further_c1_03_mad_lads_is_bought_from_m2_by_default_end_to_end() {
    let mut w = world();
    assert!(flow::api::view(C).unwrap().m2, "M2 is on for a collection the flow has no state for yet");
    let (asset, _, m1_before) = bought_on_m2(&mut w);
    let cost = flow::with(|s| s.cols[&C].note.clone());
    assert!(cost.contains("buying"), "{cost}");
    w.settle();
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("landed") && x.contains("custody is checked")), "{l:?}");
    assert!(l.iter().any(|x| x.contains("in K_main's custody: mint_item + settle_advance") && x.contains("in the books")), "{l:?}");
    assert!(in_custody(&w, &asset, &me()), "the item is in K_main's custody at finalized");
    let c = coll();
    assert_eq!((c.ledger.held, c.module.transit_units), (1, 0), "booked: held, the advance settled");
    assert!(c.ledger.held_markers.contains_key(&asset));
    let v = flow::api::view(C).unwrap();
    assert!(v.arrival.is_none() && v.held.iter().any(|(m, _, _)| *m == b58_encode(&asset)));
    assert_eq!(v.lookup_table.map(|t| t.1), Some(true));
    let paid = m1_before - w.lamports(&m1());
    assert_eq!(v.held[0].2, paid, "the advance M[1] → K_main is the purchase's all-in, booked as the item's cost");
}

/// An M2 purchase that lands and FAILS (the seller delisted between the read and the landing): one transaction, so the advance never
/// left `M[i]`; its link is undone and the books are as before (nothing in transit, the backing and revenue unchanged, no item).
#[test]
fn fixed_further_c1_03_a_failed_m2_purchase_returns_the_advance_to_the_module() {
    let mut w = world();
    let (_, module, m1_before) = bought_on_m2(&mut w);
    w.chain.borrow_mut().fail_next = Some(json!({ "InstructionError": [4, { "Custom": 6013 }] }));
    w.settle();
    let now = w.now;
    let l = block_on(flow::run::settle(&mut w, now));
    assert!(l.iter().any(|x| x.contains("failed on chain") && x.contains("undone")), "{l:?}");
    let c = coll();
    assert_eq!((c.module.transit_units, c.module.backing, c.module.revenue, c.module.surplus, c.ledger.held),
        (0, module.backing, module.revenue, module.surplus, 0), "the books are as before the advance");
    assert_eq!(w.lamports(&m1()), m1_before, "nothing left M[1]");
    assert!(flow::api::view(C).unwrap().arrival.is_none());
    assert!(state::with(|s| s.link.links.values().all(|l| l.phase == etch_autobuyer::chainlink::ledger::Phase::Failed)));
}

/// The lookup table is read at finalized before each use: one that is not `K_main`'s (or holds other addresses) is never used; the
/// M2 ask waits, nothing is booked, the operator sees why, and no second table is made over it.
#[test]
fn fixed_further_c1_03_a_lookup_table_not_as_made_is_never_used() {
    let mut w = world();
    let f = plant(&mut w, "m2_mad_lads_4vCV2b");
    fund(&mut w, 10_700_000_000);
    flow::floor::put_evidence(C, &m2_evidence(&f), &stats(10_700_000_000), w.now).unwrap();
    tick(&mut w);
    w.settle();
    let t = flow::with(|s| s.table.clone()).unwrap();
    {
        let mut c = w.chain.borrow_mut();
        let a = c.accounts.get_mut(&t.key).unwrap();
        let mut st = soltx::decode_lookup_table(&a.data).unwrap();
        st.authority = Some([0x66; 32]);
        a.data = soltx::encode_lookup_table(&st);
    }
    let signed = w.signer.signed();
    let l = tick(&mut w);
    assert!(l.iter().all(|x| !x.contains("signed and sent")), "{l:?}");
    assert_eq!(w.signer.signed(), signed, "nothing signed");
    assert!(state::with(|s| s.link.links.is_empty()) && coll().module.transit_units == 0);
    assert!(flow::with(|s| s.cols[&C].note.contains("not K_main's")), "{}", flow::with(|s| s.cols[&C].note.clone()));
    assert_eq!(flow::with(|s| s.table.clone()).map(|x| x.key), Some(t.key), "no second table");
}

/// M2 by default, the switch off at once and on again after the delay; a state saved before (M2 off) keeps its value across an upgrade.
#[test]
fn fixed_further_c1_03_m2_is_on_by_default_off_at_once_on_after_the_delay() {
    let mut w = world();
    assert!(flow::DEFAULT_M2 && flow::ColState::default().m2);
    assert!(flow::api::view(C).unwrap().m2);
    let now = w.now;
    // off: at once (a narrowing)
    flow::api::configure(C, None, None, Some(false), now).unwrap();
    let v = flow::api::view(C).unwrap();
    assert!(!v.m2 && v.pending.is_none(), "{v:?}");
    // a state saved with M2 off keeps it (an existing collection is never switched on by the new default)
    let blob = flow::encode_state(&flow::with(|s| s.clone()));
    flow::reset();
    let back = flow::decode_state(&blob).unwrap();
    assert!(!back.cols[&C].m2);
    flow::with_mut(|s| *s = back);
    // on again: waits the delay (a widening)
    let r = flow::api::configure(C, None, None, Some(true), now).unwrap();
    assert!(r.contains("applied at"), "{r}");
    let v = flow::api::view(C).unwrap();
    assert!(!v.m2 && v.pending.as_ref().is_some_and(|p| p.m2 && p.at == now + flow::CHANGE_DELAY_SECS), "{v:?}");
    w.advance_time(flow::CHANGE_DELAY_SECS - 10);
    tick(&mut w);
    assert!(!flow::api::view(C).unwrap().m2, "not before the delay");
    w.advance_time(10);
    tick(&mut w);
    let v = flow::api::view(C).unwrap();
    assert!(v.m2 && v.pending.is_none(), "{v:?}");
    // off again while a switch-on waits: at once, and the waiting one is dropped
    flow::api::configure(C, None, None, Some(false), w.now).unwrap();
    flow::api::configure(C, None, None, Some(true), w.now).unwrap();
    flow::api::configure(C, None, None, Some(false), w.now).unwrap();
    let v = flow::api::view(C).unwrap();
    assert!(!v.m2 && v.pending.is_none(), "{v:?}");
}

// ---- C1-04 (Medium)---------------------------------------------------------------------------------------------------------------------

/// C1-04. A landed purchase is booked through the deferred writer (B1-06): with a link of the collection open, `desk::book_outcome`
/// OWES `SolNftArrived` and returns `Ok`, and `buy::arrived` forgets the arrival at once. When the owed write is applied and refused
/// (here `mint_item` on a listing the operator paused meanwhile: pausing stops no burn or redemption, so a desk link runs), the
/// refusal is only an incident: nothing retries it. The item is in `K_main`'s custody but never in the books, the advance stays in
/// transit for ever (`transit_units = 1`: every later `advance` is `AdvanceOpen`), and the collection never buys again.
///
/// A fix must refuse to lose it: keep the arrival until the books HOLD the item (the flow re-checks the ledger, not `book_outcome`'s
/// `Ok`), or have `apply_owed` keep a refused `SolNftArrived` owed and retry it.
///
/// FIXED (2026-10-12): `buy::arrived` keeps the arrival until the books hold the item (its held marker): it waits while the write is
/// owed, and makes it again (custody read first) once the owed write was refused; the advance settles after the unpause.
#[test]
fn fixed_c1_04_an_owed_arrival_refused_at_apply_is_retried_and_the_advance_settles() {
    let mut w = world();
    let a = bought(&mut w);
    w.settle();
    let now = w.now;
    // the purchase landed: the flow takes the line's outcome (the advance link landed, the arrival noted; its custody is read next)
    let l = block_on(flow::run::settle(&mut w, now));
    assert!(l.iter().any(|x| x.contains("landed")), "{l:?}");
    assert!(flow::api::view(C).unwrap().arrival.is_some());
    // the operator pauses the listing (an incident elsewhere); a holder's desk burn of the collection is out on the desk line
    let burn = state::with_mut(|s| {
        s.books.set_paused(C, true).unwrap();
        let spec = Spec { key: "desk:burn:1".into(), kind: OpKind::Burn, coll: Some(C), intent: Some(1), scope: Scope::Collection };
        s.link.begin(&mut s.books, spec, now, |_| Ok((Effects { burn_units: ONE, lamports_out: 1, ..Default::default() }, 0))).unwrap().0
    });
    // the purchase's outcome: custody confirmed at finalized, the books' write owed under the open link; the arrival is KEPT
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("in K_main's custody: mint_item + settle_advance") && x.contains("owed")), "{l:?}");
    assert!(flow::api::view(C).unwrap().arrival.is_some(), "kept until the books hold the item");
    assert_eq!(state::with(|s| s.link.owed_of(C).len()), 1);
    assert!(tick(&mut w).iter().all(|x| !x.contains("owed")), "an owed write is waited for, never written twice");
    assert_eq!(state::with(|s| s.link.owed_of(C).len()), 1);
    // the burn lands: the owed write is applied, refused (the listing is paused), an incident; the arrival is still there
    state::with_mut(|s| s.link.landed(&mut s.books, burn, "burn-sig", now));
    assert!(state::with(|s| s.link.incidents.iter().any(|i| i.what.contains("mint_item") && i.what.contains("is refused"))));
    assert!(state::with(|s| s.link.owed_of(C).is_empty()));
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("the books refuse it now")), "made again, refused while paused: {l:?}");
    assert!(flow::api::view(C).unwrap().arrival.is_some());
    // the operator unpauses: the next pass books the item and settles the advance
    state::with_mut(|s| s.books.set_paused(C, false).unwrap());
    w.advance_time(600);
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("in K_main's custody: mint_item + settle_advance") && x.contains("in the books")), "{l:?}");
    let c = coll();
    assert!(in_custody(&w, &a, &me()));
    assert!(c.ledger.held_markers.contains_key(&a) && c.ledger.held == 1, "in the books");
    assert_eq!(c.module.transit_units, 0, "the advance settled");
    let v = flow::api::view(C).unwrap();
    assert!(v.arrival.is_none() && v.held.len() == 1);
}

/// C1-04: an owed arrival applied when the link is decided is seen as booked at the next pass (the arrival cleared, the item held).
#[test]
fn fixed_c1_04_an_owed_arrival_applied_later_clears_the_arrival() {
    let mut w = world();
    let a = bought(&mut w);
    w.settle();
    let now = w.now;
    block_on(flow::run::settle(&mut w, now));
    let burn = state::with_mut(|s| {
        let spec = Spec { key: "desk:burn:1".into(), kind: OpKind::Burn, coll: Some(C), intent: Some(1), scope: Scope::Collection };
        s.link.begin(&mut s.books, spec, now, |_| Ok((Effects { burn_units: ONE, lamports_out: 1, ..Default::default() }, 0))).unwrap().0
    });
    tick(&mut w);
    assert!(flow::api::view(C).unwrap().arrival.is_some());
    state::with_mut(|s| s.link.landed(&mut s.books, burn, "burn-sig", now));
    assert!(coll().ledger.held_markers.contains_key(&a), "the owed write applied");
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("is in the books")), "{l:?}");
    let v = flow::api::view(C).unwrap();
    assert!(v.arrival.is_none() && v.held.len() == 1);
}

// ---- C1-05 (Medium) ---------------------------------------------------------------------------------------------------------------------

/// C1-05 (no PoC in the audit): the floor and the purchase are the hand-in's choice; a floor walked up 20 % a day on chosen listings
/// carried every buy with it.
///
/// FIXED (2026-10-12): the buy's limit is at most the LOWEST floor the canister accepted over `FLOOR_WINDOW_DAYS` (7) days plus the
/// premium (`buy::window_limit`, the floors kept by day in `ColState::floor_days`): a walked-up floor buys nothing above the window's
/// low until it has held a week.
#[test]
fn fixed_c1_05_a_floor_walked_up_carries_no_buy_above_the_weeks_low() {
    assert_eq!(buy::window_limit(11 * SOL, 9 * SOL, 1_000), 9_900_000_000);
    assert_eq!(buy::window_limit(9 * SOL, 10 * SOL, 1_000), 9 * SOL, "never above the limit it bounds");
    let mut s = flow::ColState::default();
    s.note_floor(9 * SOL, 0);
    s.note_floor(8 * SOL, 3_600);
    s.note_floor(10 * SOL, 86_400);
    assert_eq!(s.window_floor(11 * SOL, 2 * 86_400), 8 * SOL, "the day's lowest, kept");
    assert_eq!(s.window_floor(11 * SOL, 8 * 86_400 - 1), 8 * SOL);
    assert_eq!(s.window_floor(11 * SOL, 8 * 86_400), 10 * SOL, "older than the window: forgotten");

    // the books' floor is 10.7 SOL (walked up), but the flow accepted 9 SOL two days ago: the 10.62 SOL Tensor ask is over 9.9 SOL
    let mut w = world();
    let f = plant(&mut w, "tensor_pnft_mad_lads");
    fund(&mut w, 10_700_000_000);
    let now = w.now;
    flow::with_mut(|s| s.cols.entry(C).or_default().note_floor(9 * SOL, now - 2 * 86_400));
    flow::floor::put_evidence(C, &tensor_evidence(&f), &stats(10_700_000_000), w.now).unwrap();
    let signed = w.signer.signed();
    tick(&mut w);
    assert_eq!(w.signer.signed(), signed, "nothing bought");
    let note = flow::api::view(C).unwrap().note;
    assert!(note.contains("no verified ask within 9900000000") && note.contains("7-day floor 9000000000"), "{note}");
    // the low is a week old: forgotten, the buy goes out
    flow::with_mut(|s| s.cols.get_mut(&C).unwrap().floor_days = Some(vec![((now - 8 * 86_400) / 86_400, 9 * SOL)]));
    w.advance_time(10);
    flow::floor::put_evidence(C, &tensor_evidence(&f), &stats(10_700_000_000), w.now).unwrap();
    assert!(tick(&mut w).iter().any(|x| x.contains("signed and sent")));
}

// ---- C1-06 (Low) ------------------------------------------------------------------------------------------------------------------------

/// C1-06 (no PoC in the audit): the floor post ignored `collection_busy`; a link's undo wiped it.
///
/// FIXED (2026-10-12): `floor::pass` reads nothing and posts nothing while a link of the collection is open (review F1); the evidence
/// stays unread and is posted once the collection is free.
#[test]
fn fixed_c1_06_no_floor_is_posted_while_a_link_of_the_collection_is_open() {
    let mut w = world();
    let f = plant(&mut w, "floor_mad_lads");
    let now = w.now;
    let busy = state::with_mut(|s| {
        let spec = Spec { key: "desk:burn:1".into(), kind: OpKind::Burn, coll: Some(C), intent: Some(1), scope: Scope::Collection };
        s.link.begin(&mut s.books, spec, now, |_| Ok((Effects { burn_units: ONE, lamports_out: 1, ..Default::default() }, 0))).unwrap().0
    });
    flow::floor::put_evidence(C, &f["me_listings"].to_string(), &f["me_stats"].to_string(), w.now).unwrap();
    let l = tick(&mut w);
    assert!(l.iter().all(|x| !x.contains("post_floor")), "{l:?}");
    assert_eq!((coll().module.accepted, flow::api::view(C).unwrap().read_at), (0, 0), "nothing posted, the evidence unread");
    state::with_mut(|s| s.link.landed(&mut s.books, busy, "burn-sig", now));
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("post_floor(8353000000 lamports")), "{l:?}");
    assert_eq!(coll().module.accepted, 8_353_000_000);
}

// ---- C1-07 (Low) ------------------------------------------------------------------------------------------------------------------------

/// C1-07 (no PoC in the audit): any simulation refusal (but `insufficient_funds`) counted toward `DELIVERY_TRIES`.
///
/// FIXED (2026-10-12): only a refusal AT the delivery's own instructions (`deliver::names_destination`: the message index of the
/// transfer, the durable nonce's advance before it) counts; one at the nonce (or anything not naming an instruction) is a wait.
#[test]
fn fixed_c1_07_a_refusal_that_is_not_the_destinations_never_counts() {
    let ixs = vec![soltx::compute_unit_limit(1), soltx::transfer([1; 32], [2; 32], 1)];
    let err = |i: usize| format!("{}: error {{\"InstructionError\":[{i},{{\"Custom\":1}}]}}; nothing was signed", etch_autobuyer::solsend::SIMULATION_REFUSED);
    assert_eq!(deliver::failed_instruction(&err(12)), Some(12));
    assert!(!deliver::names_destination(&err(0), &ixs, 1), "the nonce's advance");
    assert!(!deliver::names_destination(&err(1), &ixs, 1), "the compute limit");
    assert!(deliver::names_destination(&err(2), &ixs, 1), "the transfer");
    assert!(!deliver::names_destination(&err(3), &ixs, 1), "no such instruction");
    assert!(!deliver::names_destination("BlockhashNotFound", &ixs, 1));

    let mut w = world();
    let a = held(&mut w);
    let t = redeem_and_resolve(&mut w, 1, Destination::Sol(pk(KeyPath::Intent(77))));
    NONCE_FAULT.with(|f| *f.borrow_mut() = true);
    let mut all = vec![];
    for _ in 0..flow::DELIVERY_TRIES + 2 {
        all.extend(tick(&mut w));
    }
    assert!(all.iter().filter(|x| x.contains("waits") && x.contains("InstructionError")).count() >= flow::DELIVERY_TRIES as usize, "{all:?}");
    assert!(all.iter().all(|x| !x.contains("refused (") && !x.contains("undeliverable")), "{all:?}");
    assert!(!coll().ledger.tickets[&t].undeliverable);
    assert_eq!(deliver::tries(C, t), 0);
    // the fault clears: delivered
    NONCE_FAULT.with(|f| *f.borrow_mut() = false);
    tick(&mut w);
    w.settle();
    tick(&mut w);
    assert_eq!(coll().ledger.tickets[&t].stage, Stage::Delivered);
    assert!(!in_custody(&w, &a, &me()));
}

// ---- C1-08 (Low) ------------------------------------------------------------------------------------------------------------------------

/// C1-08 (no PoC in the audit): a delivery signed for a ticket past its late window raced the public refund.
///
/// FIXED (2026-10-12): no delivery is signed within `LATE_MARGIN_SECS` (a day) of the ticket's late refund, or past it: the refund rule
/// decides.
#[test]
fn fixed_c1_08_no_delivery_is_signed_near_or_past_the_late_refund() {
    let mut w = world();
    let a = held(&mut w);
    let t = redeem_and_resolve(&mut w, 1, Destination::Sol(pk(KeyPath::Intent(77))));
    let refund_at = coll().ledger.tickets[&t].deliver_by + LATE_DELIVERY_WINDOW;
    w.advance_time((refund_at - flow::LATE_MARGIN_SECS / 2 - w.now as i64) as u64);
    let signed = w.signer.signed();
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains(&format!("ticket {t} is within")) && x.contains("late refund")), "{l:?}");
    assert_eq!(w.signer.signed(), signed, "nothing signed");
    w.advance_time(flow::LATE_MARGIN_SECS as u64);
    let now = w.now as i64;
    assert!(state::with_mut(|s| s.books.refund(C, t, true, now)).is_ok(), "the refund rule decides");
    assert!(in_custody(&w, &a, &me()), "and the item never left");
    assert_eq!(state::with_mut(|s| s.books.refund(C, t, true, now)), Err(BooksError::WrongStage));
}

// ---- Informational ----------------------------------------------------------------------------------------------------------------------

/// C1-I1: the narrowing half of a combined change applies at once; only the widening half waits.
#[test]
fn fixed_c1_i1_a_narrower_premium_with_m2_switched_on_narrows_at_once() {
    let _w = world();
    // M2 is on by default since 2026-10-12: switched off first (at once), then on again with the narrower premium
    flow::api::configure(C, None, None, Some(false), 1_000).unwrap();
    let r = flow::api::configure(C, None, Some(500), Some(true), 1_000).unwrap();
    assert!(r.contains("premium 500 bps: at once") && r.contains("applied at"), "{r}");
    let s = flow::with(|s| s.cols[&C].clone());
    assert_eq!((s.premium_bps, s.m2, s.pending.as_ref().map(|p| (p.premium_bps, p.m2))), (500, false, Some((500, true))));
}

/// The ColState of a build before the fix round (with the hand-in and the asks in the stable blob).
#[derive(CandidType, Deserialize, Clone)]
struct OldCol {
    standard: Standard,
    premium_bps: u16,
    m2: bool,
    pending: Option<flow::PendingChange>,
    evidence: Option<flow::Evidence>,
    read_at: u64,
    floor: Option<etch_autobuyer::solnftfloor::SolFloor>,
    floor_at: u64,
    floor_note: String,
    asks: Vec<(nbuy::Ask, solnft::Member)>,
    arrival: Option<flow::Arrival>,
    tries: BTreeMap<u64, u32>,
    held: BTreeMap<Pubkey, flow::Held>,
    note: String,
}

#[derive(CandidType, Deserialize, Clone)]
struct OldState {
    cols: BTreeMap<u16, OldCol>,
    out: Option<etch_autobuyer::solhome::LinePending>,
    ops: u64,
    day: u64,
    signed_today: u32,
    off: bool,
}

#[derive(CandidType, Deserialize, Clone)]
enum OldStable {
    V1(OldState),
}

/// C1-I2: the stable blob holds no `solnftbuy` / `solnft` type any more (the hand-in and the asks are a transient cache), and a blob of
/// the build before (with them) still decodes.
#[test]
fn fixed_c1_i2_the_asks_are_not_persisted_and_an_older_blob_decodes() {
    let mut w = world();
    held(&mut w);
    let f = plant(&mut w, "floor_mad_lads");
    w.advance_time(10);
    flow::floor::put_evidence(C, &f["me_listings"].to_string(), &f["me_stats"].to_string(), w.now).unwrap();
    tick(&mut w);
    assert!(!flow::api::view(C).unwrap().asks.is_empty());
    let s = flow::with(|s| s.clone());
    let back = flow::decode_state(&flow::encode_state(&s)).unwrap();
    assert_eq!(back, s);
    // an upgrade: the state restored, the cache empty (the next hand-in is awaited)
    flow::reset();
    flow::with_mut(|x| *x = back);
    let v = flow::api::view(C).unwrap();
    assert!(v.asks.is_empty() && v.evidence_at == 0 && v.held.len() == 1);

    // the older blob, its asks and hand-in included
    let a: solnft::Accounts = w.chain.borrow().accounts.iter().map(|(k, v)| (*k, Some(v.clone()))).collect();
    let fm = plant(&mut w, "m2_mad_lads_4vCV2b");
    let a2: solnft::Accounts = a.into_iter().chain(w.chain.borrow().accounts.iter().map(|(k, v)| (*k, Some(v.clone())))).collect();
    let asset = fact(&fm, "asset");
    let m = solnft::is_member(&asset, &key(MAD_LADS), Standard::Programmable, &a2).unwrap();
    let cand = nbuy::Candidate { venue: nbuy::Venue::M2, listing: fact(&fm, "seller_trade_state"), asset };
    let ask = nbuy::verify_ask(&cand, &m, &a2, w.now as i64).unwrap();
    let cs = s.cols[&C].clone();
    let old = OldStable::V1(OldState { cols: [(C, OldCol { standard: cs.standard, premium_bps: cs.premium_bps, m2: cs.m2, pending: None,
        evidence: Some(flow::Evidence { cands: vec![cand], reference: 1, at: 2 }), read_at: cs.read_at, floor: cs.floor.clone(), floor_at: cs.floor_at,
        floor_note: cs.floor_note.clone(), asks: vec![(ask, m)], arrival: None, tries: cs.tries.clone(), held: cs.held.clone(), note: cs.note.clone() })]
        .into_iter().collect(), out: None, ops: s.ops, day: s.day, signed_today: s.signed_today, off: s.off });
    let decoded = flow::decode_state(&candid::encode_one(old).unwrap()).unwrap();
    let c = &decoded.cols[&C];
    assert_eq!((c.held.clone(), c.read_at, c.floor_days.clone(), decoded.rate), (cs.held.clone(), cs.read_at, None, None));
}

/// C1-I5: a standard switch drops the asks verified under the old standard (and the hand-in); nothing is bought from them.
#[test]
fn fixed_c1_i5_a_standard_switch_drops_the_cached_asks() {
    let mut w = world();
    let f = plant(&mut w, "tensor_pnft_mad_lads");
    fund(&mut w, 10_700_000_000);
    flow::floor::put_evidence(C, &tensor_evidence(&f), &stats(10_700_000_000), w.now).unwrap();
    let now = w.now;
    // read, not bought (the flow is off), then the standard switched
    flow::with_mut(|s| s.off = true);
    tick(&mut w);
    assert!(!flow::api::view(C).unwrap().asks.is_empty());
    flow::api::configure(C, Some(Standard::Core), None, None, now).unwrap();
    let v = flow::api::view(C).unwrap();
    assert!(v.asks.is_empty() && v.evidence_at == 0 && v.read_at == 0, "{v:?}");
    flow::with_mut(|s| s.off = false);
    let signed = w.signer.signed();
    tick(&mut w);
    assert_eq!(w.signer.signed(), signed, "nothing bought from asks of the old standard");
}
