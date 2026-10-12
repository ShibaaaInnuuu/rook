//! Exploration of M2's notary rule (docs/v9/spikes/s5-magic-eden-m2.md): the same buyer-only purchase under varied conditions.
mod common;
use common::*;
use etch_autobuyer::solnft::{self, Standard};
use etch_autobuyer::solnftbuy::{self as buy, Candidate, Venue};
use etch_autobuyer::solnftm2 as m2;

fn setup(name: &str) -> (Fixture, solnft::Member, buy::Ask, m2::AuctionHouse) {
    let f = fixture(name);
    let std = Standard::Legacy; // a Token Metadata listing accepts both kinds of member
    let view = f.view();
    let asset = f.fact("asset");
    let member = solnft::is_member(&asset, &f.fact("collection"), std, &view).expect("member");
    let ask = buy::verify_ask(&Candidate { venue: Venue::M2, listing: f.fact("seller_trade_state"), asset }, &member, &view, f.unix_time).expect("ask");
    let ahacct = view.get(&soltx_key(buy::M2_AUCTION_HOUSE)).unwrap().as_ref().unwrap();
    (f, member, ask, m2::decode_auction_house(&ahacct.data).unwrap())
}
fn with_house(ask: &buy::Ask, ah: &m2::AuctionHouse) -> buy::Ask {
    let mut a = ask.clone();
    if let buy::AskDetail::M2 { house, .. } = &mut a.detail { *house = Some(ah.clone()); }
    a
}
fn soltx_key(s: &str) -> etch_autobuyer::soltx::Pubkey { etch_autobuyer::soltx::key(s) }

#[test]
fn explore() {
    let (f, member, ask, ah) = setup(&std::env::var("M2_FIXTURE").unwrap_or("m2_mad_lads_4vCV2b".into()));
    println!("AH {:?}", ah);
    for (label, nprob, req, slot_add) in [("as is", None, None, 0u64), ("nprob 0", Some(0u8), None, 0), ("nprob 255", Some(255), None, 0), ("req=0", None, Some(0u8), 0)] {
        let mut f2 = fixture(&std::env::var("M2_FIXTURE").unwrap_or("m2_mad_lads_4vCV2b".into()));
        let ahk = soltx_key(buy::M2_AUCTION_HOUSE);
        let a = f2.accounts.iter_mut().find(|(k, _)| *k == ahk).unwrap();
        if let Some(n) = nprob { a.1.data[177] = n; }
        if let Some(r) = req { a.1.data[176] = r; }
        let mut e = env(&f2);
        let _ = slot_add;
        let ixs = m2::buy_instructions(&e.me(), &ask, &member).unwrap();
        let r = e.run(&ixs, &[]);
        match r { Ok(m) => println!("{label}: OK cu {}", m.compute_units_consumed), Err(x) => println!("{label}: ERR {}", x.lines().take(1).chain(x.lines().filter(|l| l.contains("log:") || l.contains("failed")).take(4)).collect::<Vec<_>>().join(" | ")) }
    }
}

fn run_variant(fx: &str, nprob: u8, req: u8, bp: u16, taker: u16, maker: i16, slot: Option<u64>) -> String {
    let (_f0, member, ask, ah) = setup(fx);
    let mut f2 = fixture(fx);
    let ahk = soltx_key(buy::M2_AUCTION_HOUSE);
    let a = f2.accounts.iter_mut().find(|(k, _)| *k == ahk).unwrap();
    a.1.data[177] = nprob; a.1.data[176] = req;
    if let Some(s) = slot { f2.slot = s; f2.unix_time += (s as i64) / 2 % 10_000; }
    let mut e = env(&f2);
    let mut ixs = m2::buy_instructions(&e.me(), &ask, &member).unwrap();
    ixs[2].data[32..34].copy_from_slice(&bp.to_le_bytes());
    let n = ixs.len() - 1;
    if member.standard == Standard::Programmable {
        ixs[n].data[16..18].copy_from_slice(&maker.to_le_bytes());
        ixs[n].data[18..20].copy_from_slice(&taker.to_le_bytes());
    } else {
        ixs[n].data[42..44].copy_from_slice(&maker.to_le_bytes());
        ixs[n].data[44..46].copy_from_slice(&taker.to_le_bytes());
    }
    match e.run(&ixs, &[]) { Ok(_) => "OK".into(), Err(x) => x.lines().next().unwrap().to_string() + " " + &x.lines().filter(|l| l.contains("Error") || l.contains("failed") ).take(2).collect::<Vec<_>>().join(" | ") }
}

#[test]
fn grid() {
    let fx = "m2_mad_lads_4vCV2b";
    for (nprob, req) in [(79u8, 1u8), (0, 1), (255, 1), (79, 0)] {
        for bp in [10000u16, 5000, 0] {
            for taker in [200u16, 100, 0] {
                println!("nprob {nprob} req {req} bp {bp} taker {taker}: {}", run_variant(fx, nprob, req, bp, taker, 0, None));
            }
        }
    }
}

#[test]
fn controls() {
    let fx = "m2_mad_lads_4vCV2b";
    let (_f, member, ask, ah) = setup(fx);
    let f2 = fixture(fx);
    // A: a wrong notary, not signing
    {
        let mut e = env(&f2);
        let mut ixs = m2::buy_instructions(&e.me(), &ask, &member).unwrap();
        let wrong = etch_autobuyer::soltx::key("SysvarRent111111111111111111111111111111111");
        for ix in ixs.iter_mut().skip(1) { for a in ix.accounts.iter_mut() { if a.key == ah.notary { a.key = wrong; } } }
        println!("wrong notary non-signer: {:?}", e.run(&ixs, &[]).map(|m| m.compute_units_consumed).map_err(|x| x.lines().take(1).collect::<String>()));
    }
    // B: a random key as the notary and signing
    {
        let mut e = env(&f2);
        let kp = solana_keypair::Keypair::new();
        let kpk = ck(&solana_signer::Signer::pubkey(&kp));
        let mut ixs = m2::buy_instructions(&e.me(), &ask, &member).unwrap();
        for ix in ixs.iter_mut().skip(1) { for a in ix.accounts.iter_mut() { if a.key == ah.notary { a.key = kpk; a.signer = true; } } }
        let r = e.run(&ixs, &[&kp]);
        println!("random notary signing: {:?}", r.as_ref().map(|m| m.compute_units_consumed).map_err(|x| x.clone()));
    }
    // C: the real notary key in the AH replaced by a key we hold, signing
    {
        let mut f3 = fixture(fx);
        let kp = solana_keypair::Keypair::new();
        let kpk = ck(&solana_signer::Signer::pubkey(&kp));
        let ahk = soltx_key(buy::M2_AUCTION_HOUSE);
        let a = f3.accounts.iter_mut().find(|(k, _)| *k == ahk).unwrap();
        a.1.data[136..168].copy_from_slice(&kpk);
        let mut e = env(&f3);
        let mut ah2 = ah.clone(); ah2.notary = kpk;
        let mut ixs = m2::buy_instructions(&e.me(), &with_house(&ask, &ah2), &member).unwrap();
        for ix in ixs.iter_mut().skip(1) { for a in ix.accounts.iter_mut() { if a.key == kpk { a.signer = true; } } }
        println!("stand-in notary signing: {:?}", e.run(&ixs, &[&kp]).map(|m| m.compute_units_consumed).map_err(|x| x.lines().take(3).collect::<Vec<_>>().join("|")));
    }
}

#[test]
fn sweep() {
    let fx = "m2_mad_lads_4vCV2b";
    let mut ok = 0; let mut bad = vec![];
    for i in 0..300u64 {
        let r = run_variant(fx, 79, 1, 10000, 200, 0, Some(455_300_000 + i * 7919));
        if r == "OK" { ok += 1 } else { bad.push((i, r)) }
    }
    println!("sweep: {ok} OK of 300; failures {:?}", &bad[..bad.len().min(3)]);
}

#[test]
fn every_fixture() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../canister-rs/tests/fixtures/solnft");
    let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("m2_") && n != "m2_pnft_mad_lads_wallet.json").map(|n| n.trim_end_matches(".json").to_string()).collect();
    names.sort();
    for n in names {
        println!("{n}: {}", run_variant(&n, 79, 1, 10000, 200, 0, None));
    }
}

#[test]
fn one_failure() {
    let fx = std::env::var("M2_FIXTURE").unwrap();
    let (_f, member, ask, ah) = setup(&fx);
    let f2 = fixture(&fx);
    let mut e = env(&f2);
    let ixs = m2::buy_instructions(&e.me(), &ask, &member).unwrap();
    println!("MEMBER {:?} {:?} {} {:?}", member.standard, member.rule_set.map(|k| etch_autobuyer::soltx::b58_encode(&k)), member.royalty_bps, member.creators.iter().map(|(k,p)| (etch_autobuyer::soltx::b58_encode(k), *p)).collect::<Vec<_>>());
    println!("{:?}", e.run(&ixs, &[]).map(|m| m.compute_units_consumed).map_err(|x| x.lines().next().unwrap().to_string()));
}

#[test]
fn grid_legacy() {
    let fx = std::env::var("M2_FIXTURE").unwrap();
    for (nprob, req) in [(79u8, 1u8), (255, 1), (0, 1)] {
        for bp in [10000u16, 5000, 0] {
            for (taker, maker) in [(200u16, 0i16), (0, 0), (500, 0), (200, 100), (200, -100)] {
                println!("G nprob {nprob} req {req} bp {bp} taker {taker} maker {maker}: {}", run_variant(&fx, nprob, req, bp, taker, maker, None));
            }
        }
    }
}
