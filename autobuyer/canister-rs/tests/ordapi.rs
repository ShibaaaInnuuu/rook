//! The two-provider custody read (docs/v6/ordinals-apis.md §3) as the flows see it: what `ordapi::verified` hands `deposits::check_output`
//! and `custody::frog_at` when the providers agree, disagree, when one is down, and across a reorg. No network: the providers'
//! replies are the real bodies captured on 2026-09-23 (ord, Ordinals Wallet) and UniSat's swagger shape.

use etch_autobuyer::custody::{self, Contents, Location, NotSendable, Outpoint, Output};
use etch_autobuyer::deposits;
use etch_autobuyer::ordapi::{self, Answer, Kind, Provider};

const ID: &str = "3a5b201020e7a3ed48bdc77e25559bbdebd5926904b9b46301c79481476fa703i0";
const OUT: &str = "579526a20898e3ae9f279241f4fd5c06d9439c1fe2528dc62ddb6e295c2a0ec0:0";
const H: u64 = 968_323;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/ordapi/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}
fn outpoint() -> Outpoint {
    Outpoint::parse(OUT).unwrap()
}
fn custody_script() -> Vec<u8> {
    let mut s = vec![0x51, 0x20];
    s.extend([0xc2; 32]);
    s
}
fn unisat_inscription(txid: &str) -> Vec<u8> {
    format!(r#"{{"code":0,"msg":"ok","data":{{"inscriptionId":"{ID}","offset":0,"height":780131,"utxo":{{"txid":"{txid}","vout":0,"satoshi":10000}}}}}}"#).into_bytes()
}
fn unisat_utxo(ids: &[&str]) -> Vec<u8> {
    let list: Vec<String> = ids.iter().map(|i| format!(r#"{{"inscriptionId":"{i}","offset":0}}"#)).collect();
    format!(r#"{{"code":0,"msg":"ok","data":{{"txid":"{}","vout":0,"satoshi":10000,"inscriptions":[{}],"isSpent":false}}}}"#, &OUT[..64], list.join(",")).into_bytes()
}
const NO_RUNES: &[u8] = br#"{"code":0,"msg":"ok","data":[]}"#;

/// What `ordnet::ask` builds for one provider: its height twice around the fact.
fn answer<T>(p: &Provider, h: (u64, u64), value: Result<T, String>) -> Answer<T> {
    Answer { provider: p.name.clone(), height_before: Ok(h.0), height_after: Ok(h.1), value }
}

/// The custody read as `frogflow::custody_holds` and `redeem` make it: the location from every provider, the contents from
/// those with a per-output view, each verified, then the explorers' output judged by `custody::frog_at`.
fn custody_read(location: &[Answer<Option<Location>>], contents: &[Answer<Option<Contents>>], out: &Output, spent: bool) -> Result<String, String> {
    let (loc, _) = ordapi::verified("the frog's location", location)?;
    let loc = loc.clone().ok_or("no provider knows the frog")?;
    let (c, _) = ordapi::verified("the output's contents", contents)?;
    match custody::frog_at(ID, &loc, c.as_ref(), out, spent, &custody_script()) {
        Ok(f) => Ok(format!("sendable from {} ({} sats)", f.output.text(), f.value)),
        Err(NotSendable::Wait(w)) => Err(format!("wait: {w}")),
        Err(NotSendable::Stuck(s)) => Err(format!("stuck: {s}")),
    }
}

#[test]
fn agree_the_keyless_pair_locates_the_frog_and_ord_plus_unisat_verify_its_contents() {
    let (ord, ow, uni) = (Provider::ord(ordapi::MAINNET_ORD), Provider::ordinals_wallet(), Provider::unisat());
    let loc = |p: &Provider, body: &[u8]| answer(p, (H, H), ordapi::parse_location(p.kind, 200, body, ID));
    let location = [loc(&ord, &fixture("ord-r-inscription-frog-3487.json")), loc(&ow, &fixture("ordinalswallet-inscription-frog-3487.json")),
        loc(&uni, &unisat_inscription(&OUT[..64]))];
    let contents = [answer(&ord, (H, H), ordapi::parse_contents(Kind::Ord, 200, &fixture("ord-r-utxo-frog-3487.json"), None)),
        answer(&uni, (H, H), ordapi::parse_contents(Kind::UniSat, 200, &unisat_utxo(&[ID]), Some((200, NO_RUNES))))];
    let out = Output { script: custody_script(), value: 10_000, confirmed: true };
    assert_eq!(custody_read(&location, &contents, &out, false).unwrap(), format!("sendable from {OUT} (10000 sats)"));
    // the same facts through the deposit judge: exactly one inscription, no runes, here, at custody
    let (l, _) = ordapi::verified("l", &location).unwrap();
    let (c, _) = ordapi::verified("c", &contents).unwrap();
    assert_eq!(deposits::check_output(&outpoint(), c.as_ref(), l.as_ref(), &out, false, &custody_script()), Ok(Some(ID.to_string())));
    // the keyless pair alone verifies the location but not the contents: one provider with a per-output view is not a check
    let keyless_contents = [contents[0].clone()];
    let e = custody_read(&location[..2], &keyless_contents, &out, false).unwrap_err();
    assert_eq!(e, "unverified: 1 of 2 providers answer the output's contents");
}

#[test]
fn disagree_is_unverified_and_the_frog_neither_moves_nor_mints() {
    let (ord, uni) = (Provider::ord(ordapi::MAINNET_ORD), Provider::unisat());
    let elsewhere = format!("{}", "ab".repeat(32));
    let location = [answer(&ord, (H, H), ordapi::parse_location(Kind::Ord, 200, &fixture("ord-r-inscription-frog-3487.json"), ID)),
        answer(&uni, (H, H), ordapi::parse_location(Kind::UniSat, 200, &unisat_inscription(&elsewhere), ID))];
    let contents = [answer(&ord, (H, H), ordapi::parse_contents(Kind::Ord, 200, &fixture("ord-r-utxo-frog-3487.json"), None)),
        answer(&uni, (H, H), ordapi::parse_contents(Kind::UniSat, 200, &unisat_utxo(&[ID]), Some((200, NO_RUNES))))];
    let out = Output { script: custody_script(), value: 10_000, confirmed: true };
    let e = custody_read(&location, &contents, &out, false).unwrap_err();
    assert!(e.starts_with("unverified: ord and unisat disagree on the frog's location"), "{e}");
    // agreed location, disagreeing contents (UniSat sees a second inscription on the output): unverified too
    let location = [location[0].clone(), answer(&uni, (H, H), ordapi::parse_location(Kind::UniSat, 200, &unisat_inscription(&OUT[..64]), ID))];
    let two = [contents[0].clone(), answer(&uni, (H, H), ordapi::parse_contents(Kind::UniSat, 200, &unisat_utxo(&[ID, &format!("{}i0", "cd".repeat(32))]), Some((200, NO_RUNES))))];
    let e = custody_read(&location, &two, &out, false).unwrap_err();
    assert!(e.starts_with("unverified: ord and unisat disagree on the output's contents"), "{e}");
    // and the deposit judge never sees a verdict: an unverified read is the "ord has not indexed" wait it already has
    assert!(matches!(deposits::check_output(&outpoint(), None, None, &out, false, &custody_script()), Err(Err(_))));
}

#[test]
fn a_provider_down_or_rate_limited_is_unverified_not_the_other_ones_word() {
    let (ord, ow) = (Provider::ord(ordapi::MAINNET_ORD), Provider::ordinals_wallet());
    let good = answer(&ord, (H, H), ordapi::parse_location(Kind::Ord, 200, &fixture("ord-r-inscription-frog-3487.json"), ID));
    for (status, body) in [(503u16, &b""[..]), (429, b"rate limited"), (200, b"<html>")] {
        let bad = answer(&ow, (H, H), ordapi::parse_location(Kind::OrdinalsWallet, status, body, ID));
        let e = ordapi::verified("the frog's location", &[good.clone(), bad]).unwrap_err();
        assert!(e.starts_with("unverified: ordinalswallet: "), "{e}");
    }
    // a provider whose height cannot be read is down too, whatever its fact said
    let mut no_height = answer(&ow, (H, H), ordapi::parse_location(Kind::OrdinalsWallet, 200, &fixture("ordinalswallet-inscription-frog-3487.json"), ID));
    no_height.height_after = Err("HTTP 502 for the height".into());
    assert_eq!(ordapi::verified("x", &[good, no_height]).unwrap_err(), "unverified: ordinalswallet: HTTP 502 for the height");
}

#[test]
fn a_reorg_or_a_stale_indexer_is_unverified_until_the_providers_are_at_one_height_again() {
    let (ord, ow) = (Provider::ord(ordapi::MAINNET_ORD), Provider::ordinals_wallet());
    let at = |p: &Provider, h: (u64, u64), body: &[u8]| answer(p, h, ordapi::parse_location(p.kind, 200, body, ID));
    let (ord_body, ow_body) = (fixture("ord-r-inscription-frog-3487.json"), fixture("ordinalswallet-inscription-frog-3487.json"));
    // one provider two blocks behind (or on the other side of a reorg): not the same question
    let e = ordapi::verified("the frog's location", &[at(&ord, (H, H), &ord_body), at(&ow, (H - 2, H - 2), &ow_body)]).unwrap_err();
    assert_eq!(e, "unverified: ord at 968323, ordinalswallet at 968321 are 2 blocks apart on the frog's location");
    // a block arriving mid-read: the provider's before and after heights differ, and its fact belongs to neither
    let e = ordapi::verified("the frog's location", &[at(&ord, (H, H + 1), &ord_body), at(&ow, (H, H), &ow_body)]).unwrap_err();
    assert_eq!(e, "unverified: ord moved from 968323 to 968324 during the read of the frog's location");
    // one block apart is a boundary, not a reorg: verified at the lower height
    let (_, h) = ordapi::verified("x", &[at(&ord, (H + 1, H + 1), &ord_body), at(&ow, (H, H), &ow_body)]).unwrap();
    assert_eq!(h, H);
    // after the reorg settles the frog may sit elsewhere: both must say so, and the old outpoint's contents read as spent (None) on both
    let moved = format!(r#"{{"charms":[],"content_type":"image/webp","content_length":3744,"delegate":null,"fee":4304,"height":780131,"id":"{ID}","number":395887,"output":"{o}","sat":1,"satpoint":"{o}:0","timestamp":1,"value":10000}}"#, o = format!("{}:1", "ee".repeat(32)));
    let ow_moved = format!(r#"{{"id":"{ID}","num":395887,"address":"bc1p","value":10000,"satpoint":"{}:1:0"}}"#, "ee".repeat(32));
    let settled = [at(&ord, (H + 3, H + 3), moved.as_bytes()), at(&ow, (H + 3, H + 3), ow_moved.as_bytes())];
    let (l, _) = ordapi::verified("x", &settled).unwrap();
    assert_eq!(l.as_ref().unwrap().output.text(), format!("{}:1", "ee".repeat(32)));
    let spent = [answer(&ord, (H + 3, H + 3), ordapi::parse_contents(Kind::Ord, 404, b"output x not found", None)),
        answer(&Provider::unisat(), (H + 3, H + 3), ordapi::parse_contents(Kind::UniSat, 200, &String::from_utf8(unisat_utxo(&[ID])).unwrap().replace("false", "true").into_bytes(), Some((200, NO_RUNES))))];
    assert_eq!(ordapi::verified("x", &spent).unwrap(), (&None, H + 3));
}
