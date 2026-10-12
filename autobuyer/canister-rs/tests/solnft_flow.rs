//! W8c: the Solana NFT flow on the books (`solnftflow`), against the fake chain (`tests/desk_common`) with REAL mainnet accounts of Mad Lads
//! (`tests/fixtures/solnft/`): the floor from Magic Eden's listings verified on chain and posted; a Tensor purchase end to end (the
//! advance booked as a link, judged, one transaction with the memo, the item in `K_main`'s custody, `mint_item` + `settle_advance`); a
//! failed landing undoes the advance; a delivery to a Solana wallet recorded; undeliverable destinations; the custody cap; a stale
//! floor; the judge's refusals. The fake executes the Tensor purchase and Token Metadata's `TransferV1` as the programs would leave
//! the accounts (their real effects are LiteSVM's: `autobuyer/solnft-svm`).
mod desk_common;
mod fakerpc;

use std::cell::RefCell;
use std::collections::BTreeMap;

use base64::Engine;
use desk_common::*;
use etch_autobuyer::books::ledger::{Destination, MintItemArgs, MintTo, Stage};
use etch_autobuyer::books::{self, ListingKind, ONE};
use etch_autobuyer::custodycap::{CapPolicy, CollectionCap};
use etch_autobuyer::desk::state;
use etch_autobuyer::solkey::KeyPath;
use etch_autobuyer::solnft::{self, Standard};
use etch_autobuyer::solnftbuy::{self as nbuy, Candidate, Expect, Venue};
use etch_autobuyer::solnftflow::{self as flow, buy, SolNftIo};
use etch_autobuyer::solrpc::Account;
use etch_autobuyer::soltx::{self, key, Instruction, Pubkey};
use serde_json::{json, Value};

const MAD_LADS: &str = "J1S9H3QjnRtBbbuD4HjPV6RpRhwuk4zKbxsnCHuTgh9w";
const C: u16 = 1;
const WX1: Pubkey = [0xC1; 32];

thread_local! {
    static CAPS: RefCell<Option<CapPolicy>> = const { RefCell::new(None) };
    /// A pNFT rule set that refuses every transfer (B2-09).
    static VETO: RefCell<bool> = const { RefCell::new(false) };
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
        Default::default()
    }
    fn outflow_check(&mut self, _: &str, _: &str, _: etch_autobuyer::outflow::Flow, _: u128) -> Result<(), String> {
        Ok(())
    }
    fn outflow_record(&mut self, _: &str, _: &str, _: etch_autobuyer::outflow::Flow, _: u128) {}
}

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

/// Tensor `buy_legacy` (pNFT) as the program leaves the accounts: the payer spends max_amount + 2 % of the listing and the rents (less the
/// escrow token record's refund), the listing and its escrow close, the buyer's ATA and token record hold the item.
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

/// Token Metadata `TransferV1` of a pNFT, wallet to wallet (or refused by the rule set when [`VETO`] is set).
fn tm_transfer(ix: &Instruction, acc: &mut BTreeMap<Pubkey, Account>) -> Result<(), Value> {
    if VETO.with(|v| *v.borrow()) {
        return Err(bad("the rule set refuses the transfer"));
    }
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

/// The fake hands the closures the instructions after the durable nonce's advance; Solana names an error by the MESSAGE's index (audit
/// C1-07 reads it): instruction `i` here is `i + 1` there.
fn at(i: usize, mut e: Value) -> Value {
    if let Some(x) = e.get_mut("InstructionError").and_then(|a| a.get_mut(0)) {
        *x = json!(i + 1);
    }
    e
}

fn exec(ixs: &[Instruction], acc: &mut BTreeMap<Pubkey, Account>) -> Result<(), Value> {
    for (i, ix) in ixs.iter().enumerate() {
        if ix.program == key(nbuy::TENSOR_MARKETPLACE) {
            tensor_buy(ix, acc).map_err(|e| at(i, e))?;
        } else if ix.program == key(solnft::TOKEN_METADATA) && ix.data.first() == Some(&49) {
            tm_transfer(ix, acc).map_err(|e| at(i, e))?;
        } else {
            run_ixs(std::slice::from_ref(ix), acc).map_err(|e| at(i, e))?;
        }
    }
    Ok(())
}

/// A world with collection 0 (an Ethereum one, the desk's) and collection 1, Mad Lads, a Solana NFT listing held by `K_main`.
fn world() -> World {
    let w = World::new();
    flow::reset();
    CAPS.with(|c| *c.borrow_mut() = None);
    VETO.with(|v| *v.borrow_mut() = false);
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

/// A recorded scenario's accounts, planted on the fake chain; its facts.
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

/// The floor posted at `floor` lamports, and ~3 units minted at the module (its SOL at `M[1]`, its wX supply on chain).
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

/// A Tensor purchase of Mad Lads set up and signed: the asset, the advance.
fn bought(w: &mut World) -> (Pubkey, u64) {
    let f = plant(w, "tensor_pnft_mad_lads");
    fund(w, 10_700_000_000);
    flow::floor::put_evidence(C, &tensor_evidence(&f), &json!({ "floorPrice": 10_700_000_000u64 }).to_string(), w.now).unwrap();
    let l = tick(w);
    assert!(l.iter().any(|x| x.contains("advance + purchase") && x.contains("signed and sent")), "{l:?}");
    let cost = state::with(|s| s.books.get(C).unwrap().module.advanced);
    (fact(&f, "asset"), cost)
}

fn in_custody(w: &World, asset: &Pubkey, holder: &Pubkey) -> bool {
    let a: solnft::Accounts = solnft::check_keys(asset, &key(MAD_LADS), Standard::Programmable, holder).into_iter()
        .map(|k| (k, w.chain.borrow().accounts.get(&k).cloned())).collect();
    solnft::check_on(&a, asset, &key(MAD_LADS), Standard::Programmable, holder).is_ok()
}

#[test]
fn the_floor_from_magic_edens_listings_is_verified_on_chain_and_posted_in_the_books() {
    let mut w = world();
    let f = plant(&mut w, "floor_mad_lads");
    // no floor yet: the collection's public mint is closed
    assert!(state::with(|s| s.books.quote_mint(C, SOL, w.now as i64)).is_err());
    let r = flow::floor::put_evidence(C, &f["me_listings"].to_string(), &f["me_stats"].to_string(), w.now).unwrap();
    assert!(r.contains("9 candidates") || r.contains("candidates"), "{r}");
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("post_floor(8353000000 lamports")), "{l:?}");
    let c = coll();
    assert_eq!((c.module.accepted, c.module.target), (8_353_000_000, 8_353_000_000));
    assert!(c.module.fresh(w.now as i64) && c.module.depth_items >= 4 && c.module.confidence_bps >= 9_900, "{:?}", c.module.confidence_bps);
    let v = flow::api::view(C).unwrap();
    assert_eq!(v.floor.as_ref().unwrap().price, 8_353_000_000);
    assert!(v.asks.iter().all(|a| a.venue == Venue::M2 && !a.keyless), "every ask is M2: nothing is bought by default");
    // the same hand-in is read once
    assert!(tick(&mut w).iter().all(|x| !x.contains("post_floor")));
    // idempotent and bounded hand-ins
    assert!(flow::floor::put_evidence(C, &"x".repeat(flow::MAX_LISTINGS_BYTES + 1), "{}", w.now).unwrap_err().contains("refused"));
    assert!(flow::floor::put_evidence(0, &f["me_listings"].to_string(), &f["me_stats"].to_string(), w.now).unwrap_err().contains("not a Solana NFT"));
}

#[test]
fn a_stale_floor_closes_the_mint_and_the_buy_and_old_evidence_is_not_read() {
    let mut w = world();
    let f = plant(&mut w, "floor_mad_lads");
    flow::floor::put_evidence(C, &f["me_listings"].to_string(), &f["me_stats"].to_string(), w.now).unwrap();
    tick(&mut w);
    assert!(state::with(|s| s.books.quote_mint(C, SOL, w.now as i64)).is_ok());
    w.advance_time(3 * 3_600 + 1);
    let now = w.now as i64;
    assert_eq!(state::with(|s| s.books.quote_mint(C, SOL, now)), Err(books::BooksError::Stale));
    assert_eq!(state::with(|s| s.books.buy_signal(C, now)).unwrap().1, 0, "no item cap on a stale floor");
    flow::floor::put_evidence(C, &f["me_listings"].to_string(), &f["me_stats"].to_string(), w.now - flow::EVIDENCE_TTL_SECS - 1).unwrap();
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("not read")), "{l:?}");
}

#[test]
fn a_tensor_purchase_end_to_end_advance_judged_one_transaction_custody_and_the_books() {
    let mut w = world();
    let m1_before = 3 * 10_700_000_000 + RENT0;
    let (asset, cost) = bought(&mut w);
    let backed = coll().ledger.sol_backed;
    // the advance is booked as a link before anything lands: one unit in transit, its lamports out of M[1]
    let c = coll();
    assert_eq!((c.module.transit_units, c.module.advanced), (ONE, cost));
    assert_eq!(cost, 10_620_000_000 + nbuy::ATA_RENT + nbuy::TOKEN_RECORD_RENT, "the ask all in");
    let link = state::with(|s| s.link.links.values().last().cloned().unwrap());
    assert_eq!((link.kind.tag(), link.effects.lamports_out, link.coll), ("advance", cost, Some(C)));
    // ONE transaction, signed by K_main and K_coll[1]: the advance, the Tensor purchase, the memo
    let tx = w.chain.borrow().sent.last().cloned().unwrap();
    let (sigs, msg) = soltx::split_tx(&tx).unwrap();
    assert_eq!(sigs.len(), 2);
    let p = soltx::Parts::parse(msg).unwrap();
    let ixs = p.decompile(&[], &[]).unwrap();
    let t = soltx::transfer(m1(), me(), cost);
    assert!(ixs.iter().any(|i| i.program == t.program && i.data == t.data && i.accounts.iter().map(|a| a.key).eq(t.accounts.iter().map(|a| a.key))),
        "the advance M[1] → K_main");
    assert!(ixs.iter().any(|i| i.program == key(nbuy::TENSOR_MARKETPLACE)));
    assert!(ixs.iter().any(|i| i.data == link.memo.as_bytes()), "the memo {}", link.memo);
    assert!(ixs.iter().all(|i| i.program != key(soltx::COMPUTE_BUDGET_PROGRAM) || i.data[0] == 2), "only SetComputeUnitLimit (B2-01)");
    // landed: the item in K_main's custody at finalized, then mint_item + settle_advance
    w.settle();
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("in K_main's custody: mint_item + settle_advance")), "{l:?}");
    assert!(in_custody(&w, &asset, &me()));
    let c = coll();
    assert_eq!((c.ledger.held, c.module.transit_units, c.ledger.module_units, c.module.items_arrived), (1, 0, 0, 1));
    assert_eq!(c.ledger.sol_backed, backed - ONE, "the transit unit became the item's");
    let item = c.ledger.items.values().next().unwrap();
    assert_eq!(item.key, asset, "the custody proof: the mint");
    assert_eq!(c.ledger.held_markers.get(&asset), Some(&item.item_id));
    assert_eq!(w.lamports(&m1()), m1_before - cost, "the advance left M[1]");
    assert_eq!(c.ledger.expected_supply(), Some(w.chain.borrow().accounts[&WX1].data[36..44].iter().rev().fold(0u64, |a, b| a * 256 + *b as u64)));
    // the link is landed, the views show the mint
    assert!(state::with(|s| s.link.open_of(C, None).next().is_none()));
    let v = etch_autobuyer::desk::api::books_of(C, w.now as i64).unwrap();
    assert_eq!(v.items[0].key, soltx::b58_encode(&asset));
    let s = flow::api::view(C).unwrap();
    assert_eq!(s.held.len(), 1);
    assert_eq!(s.held[0].0, soltx::b58_encode(&asset));
    // nothing more to buy: the one ask is held
    assert!(tick(&mut w).iter().all(|x| !x.contains("signed and sent")));
}

#[test]
fn a_failed_landing_undoes_the_advance() {
    let mut w = world();
    let f = plant(&mut w, "tensor_pnft_mad_lads");
    fund(&mut w, 10_700_000_000);
    let module = coll().module;
    let m1_before = w.lamports(&m1());
    flow::floor::put_evidence(C, &tensor_evidence(&f), &json!({ "floorPrice": 10_700_000_000u64 }).to_string(), w.now).unwrap();
    tick(&mut w);
    assert_eq!(coll().module.transit_units, ONE);
    w.chain.borrow_mut().fail_next = Some(json!({ "InstructionError": [2, { "Custom": 6001 }] }));
    w.settle();
    let now = w.now;
    let l = block_on(flow::run::settle(&mut w, now));
    assert!(l.iter().any(|x| x.contains("failed on chain") && x.contains("undone")), "{l:?}");
    let c = coll();
    assert_eq!((c.module.transit_units, c.module.backing, c.module.revenue, c.ledger.held), (0, module.backing, module.revenue, 0));
    assert_eq!(w.lamports(&m1()), m1_before, "one transaction: nothing left M[1]");
    assert!(flow::api::view(C).unwrap().arrival.is_none());
}

#[test]
fn a_resolved_ticket_is_delivered_to_a_solana_wallet_and_recorded() {
    let mut w = world();
    let (asset, _) = bought(&mut w);
    w.settle();
    tick(&mut w);
    let to = pk(KeyPath::Intent(77));
    redeem_and_resolve(&mut w, Destination::Sol(to));
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("delivery of collection 1's ticket 0") && x.contains("signed")), "{l:?}");
    w.settle();
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("recorded delivered")), "{l:?}");
    let c = coll();
    assert_eq!(c.ledger.tickets[&0].stage, Stage::Delivered);
    assert_eq!((c.ledger.held, c.ledger.open_tickets), (0, 0));
    assert!(in_custody(&w, &asset, &to) && !in_custody(&w, &asset, &me()));
    assert!(flow::api::view(C).unwrap().held.is_empty());
}

fn redeem_and_resolve(w: &mut World, d: Destination) {
    let now = w.now as i64;
    state::with_mut(|s| {
        s.books.redeem(C, wallet(1), d, now).unwrap();
        s.books.collections.get_mut(&C).unwrap().ledger.tickets.get_mut(&0).unwrap().round = books::math::round_at(now);
        s.books.resolve(C, 0, Some([5; 32]), now).unwrap();
    });
}

#[test]
fn a_destination_that_refuses_is_marked_undeliverable() {
    // not a wallet (a PDA, off the curve): at once
    let mut w = world();
    let (asset, _) = bought(&mut w);
    w.settle();
    tick(&mut w);
    redeem_and_resolve(&mut w, Destination::Sol(solnft::metadata_address(&asset)));
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("marked undeliverable")), "{l:?}");
    assert!(coll().ledger.tickets[&0].undeliverable);
    assert!(in_custody(&w, &asset, &me()), "nothing left custody");
    // a rule set that vetoes the transfer (B2-09): refused by simulation, DELIVERY_TRIES times, then marked
    let mut w = world();
    let (asset, _) = bought(&mut w);
    w.settle();
    tick(&mut w);
    redeem_and_resolve(&mut w, Destination::Sol(pk(KeyPath::Intent(78))));
    VETO.with(|v| *v.borrow_mut() = true);
    let mut all = vec![];
    for _ in 0..flow::DELIVERY_TRIES {
        all.extend(tick(&mut w));
    }
    assert!(all.iter().any(|x| x.contains("marked undeliverable")), "{all:?}");
    assert!(coll().ledger.tickets[&0].undeliverable);
    assert!(in_custody(&w, &asset, &me()));
    // a burn is never a destination of a Solana NFT listing (the ledger refuses it)
    let now = w.now as i64;
    assert_eq!(state::with_mut(|s| s.books.redeem(C, wallet(1), Destination::Burn, now)), Err(books::BooksError::BadDestination));
}

#[test]
fn the_custody_cap_of_the_collection_holds_the_buy() {
    let mut w = world();
    let f = plant(&mut w, "tensor_pnft_mad_lads");
    fund(&mut w, 10_700_000_000);
    // one item held already, and a cap of one item
    let now = w.now as i64;
    state::with_mut(|s| {
        let args = MintItemArgs { key: [9; 32], source: books::math::keccak(&[&[9u8; 32], &[3u8; 32]]), tx: [3; 32], vout: 0, deposit: false,
            inscription_id: String::new(), proof: vec![] };
        s.books.mint_item(C, args, MintTo::Holder(wallet(1)), now).unwrap();
    });
    let k = flow::cap_key(&key(MAD_LADS));
    CAPS.with(|c| *c.borrow_mut() = Some(CapPolicy { collections: vec![CollectionCap { collection: k, max_items: 1, max_value_wei: 10u128.pow(19),
        max_item_wei: 10u128.pow(19) }], global_max_items: 1, global_max_value_wei: 10u128.pow(19) }));
    flow::floor::put_evidence(C, &tensor_evidence(&f), &json!({ "floorPrice": 10_700_000_000u64 }).to_string(), w.now).unwrap();
    let signed = w.signer.signed();
    let l = tick(&mut w);
    assert!(l.iter().all(|x| !x.contains("signed and sent")), "{l:?}");
    assert_eq!(w.signer.signed(), signed, "nothing signed");
    assert!(flow::api::view(C).unwrap().note.contains("custody cap"), "{}", flow::api::view(C).unwrap().note);
    assert_eq!(coll().module.transit_units, 0);
}

#[test]
fn the_judge_refuses_a_compute_unit_price_a_hostile_ask_and_an_ask_over_the_limit_and_nothing_is_signed() {
    // the whole transaction judged: a compute-unit price, a second transfer, an advance off the ask all in
    let mut w = world();
    let f = plant(&mut w, "tensor_pnft_mad_lads");
    let a: solnft::Accounts = w.chain.borrow().accounts.iter().map(|(k, v)| (*k, Some(v.clone()))).collect();
    let (asset, coll_key) = (fact(&f, "asset"), key(MAD_LADS));
    let m = solnft::is_member(&asset, &coll_key, Standard::Programmable, &a).unwrap();
    let ask = nbuy::verify_ask(&Candidate { venue: Venue::Tensor, listing: fact(&f, "list_state"), asset }, &m, &a, w.now as i64).unwrap();
    let amount = ask.total + ask.rent(&m);
    let e = Expect { payer: me(), ask: &ask, member: &m, collection: coll_key, limit: amount };
    let ixs = buy::purchase(&me(), &ask, &m).unwrap();
    let memo = "rook:v1:1:advance:0";
    let mut tx = vec![ixs[0].clone(), soltx::transfer(m1(), me(), amount), ixs[1].clone(), etch_autobuyer::solix2::spl::memo(memo, &[])];
    buy::judge_tx(&tx, &m1(), amount, memo, &e).unwrap();
    let mut priced = tx.clone();
    priced.insert(0, soltx::compute_unit_price(1_000_000));
    assert!(buy::judge_tx(&priced, &m1(), amount, memo, &e).unwrap_err().contains("SetComputeUnitPrice"));
    let mut twice = tx.clone();
    twice.push(soltx::transfer(m1(), me(), 1));
    assert!(buy::judge_tx(&twice, &m1(), amount, memo, &e).is_err());
    tx[1] = soltx::transfer(m1(), me(), amount + 1);
    assert!(buy::judge_tx(&tx, &m1(), amount + 1, memo, &e).is_err());
    assert!(buy::judge_tx(&tx, &m1(), amount + 1, memo, &Expect { limit: amount - 1, ..e.clone() }).is_err());
    // a hostile candidate (the listing named for another asset) is dropped on chain; nothing signed
    fund(&mut w, 10_700_000_000);
    let hostile = json!([{ "listingSource": "TENSOR_MARKETPLACE_LISTING", "pdaAddress": f["list_state"], "tokenMint": "4vCV2b3uSsLdgFBLC33reou8wv8JtfbJohqGmxJZs4e9" }]);
    flow::floor::put_evidence(C, &hostile.to_string(), &json!({ "floorPrice": 10_700_000_000u64 }).to_string(), w.now).unwrap();
    let signed = w.signer.signed();
    tick(&mut w);
    assert_eq!(w.signer.signed(), signed);
    assert!(flow::api::view(C).unwrap().asks.is_empty());
    // the real ask, but over the limit (a floor of 9 SOL: the books' item cap is under 10.62 SOL): nothing signed
    let mut w = world();
    let f = plant(&mut w, "tensor_pnft_mad_lads");
    fund(&mut w, 9 * SOL);
    flow::floor::put_evidence(C, &tensor_evidence(&f), &json!({ "floorPrice": 9 * SOL }).to_string(), w.now).unwrap();
    let signed = w.signer.signed();
    tick(&mut w);
    assert_eq!(w.signer.signed(), signed);
    assert!(flow::api::view(C).unwrap().note.contains("no verified ask within"), "{}", flow::api::view(C).unwrap().note);
    assert!(state::with(|s| s.link.links.is_empty()), "nothing booked");
}

#[test]
fn the_knobs_widen_after_a_day_and_the_state_round_trips() {
    let _w = world();
    let now = 1_000;
    assert!(flow::api::configure(C, None, Some(2_000), None, now).unwrap().contains("applied at"));
    assert_eq!(flow::with(|s| s.cols[&C].premium_bps), flow::DEFAULT_PREMIUM_BPS);
    assert!(flow::api::configure(C, None, Some(500), None, now).unwrap().contains("at once"));
    assert_eq!(flow::with(|s| s.cols[&C].premium_bps), 500);
    // M2 is on by default (2026-10-12): off at once, on again after the delay
    assert!(flow::with(|s| s.cols[&C].m2));
    assert!(flow::api::configure(C, None, None, Some(false), now).unwrap().contains("at once"));
    assert!(!flow::with(|s| s.cols[&C].m2));
    assert!(flow::api::configure(C, None, None, Some(true), now).unwrap().contains("applied at"));
    assert!(flow::api::configure(C, None, Some(flow::MAX_PREMIUM_BPS + 1), None, now).is_err());
    assert!(flow::api::configure(C, Some(Standard::Core), None, None, now).is_ok(), "the standard while nothing is held");
    let s = flow::with(|s| s.clone());
    assert_eq!(flow::decode_state(&flow::encode_state(&s)).unwrap(), s);
    assert!(flow::decode_state(b"nope").is_err());
    assert_eq!(buy::limit_of(10 * SOL, 1_000, 10_300_000_000), 10_300_000_000);
    assert_eq!(buy::limit_of(10 * SOL, 100, 20 * SOL), 10_100_000_000);
}
