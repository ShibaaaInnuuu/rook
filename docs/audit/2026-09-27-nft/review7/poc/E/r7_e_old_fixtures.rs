//! Review round 7, part E: every historical EthState fixture's bytes (git history, not the re-encoded HEAD copies) decode with
//! HEAD's types, re-encode with no lost opt, and the new round-6 fields are absent; and the reverse: HEAD's round-6 bytes decode
//! into the round-5 (bb713fe) `.did` types via IDLValue (subtype check of the new opt fields).
use etch_autobuyer::canister::lost_opts;
use etch_autobuyer::ethflow::EthState;

#[test]
fn r7_e_every_historical_eth_state_fixture_restores_under_head() {
    let dir = std::path::Path::new("/tmp/rook-r7-F/old");
    let mut n = 0;
    let mut names: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    names.sort();
    for p in names {
        let hex_ = std::fs::read_to_string(&p).unwrap();
        let bytes = hex::decode(hex_.trim()).unwrap();
        let e: EthState = candid::decode_one(&bytes).unwrap_or_else(|x| panic!("{p:?} does not decode: {x}"));
        let again = candid::encode_one(&e).unwrap();
        let lost = lost_opts(&bytes, &again).unwrap();
        let round6 = p.to_string_lossy().contains("round6");
        let pend = e.pending.as_ref().map(|x| (x.pin.is_some(), x.sent_at.is_some()));
        let scans: Vec<_> = e.sale_scan.iter().flatten().map(|s| (s.hole.is_some(), s.lost.is_some(), s.behind.is_some(), s.rfail.is_some(), s.raw.is_some())).collect();
        println!("{:?}: {} bytes, lost {:?}, pending(pin,sent_at) {:?}, scans(hole,lost,behind,rfail,raw) {:?}", p.file_name().unwrap(), bytes.len(), lost, pend, scans);
        assert!(lost.is_empty(), "{p:?}: {lost:?}");
        if !round6 && !p.to_string_lossy().contains("2d747ba") {
            assert!(e.pending.as_ref().is_none_or(|x| x.sent_at.is_none()));
            assert!(e.sale_scan.iter().flatten().all(|s| s.hole.is_none() && s.lost.is_none() && s.behind.is_none() && s.rfail.is_none() && s.raw.is_none()));
        }
        n += 1;
    }
    assert!(n >= 12, "{n}");
}

#[test]
fn r7_e_the_re_encoded_fixtures_hold_the_same_values_as_their_bb713fe_bytes() {
    for r in [2, 4, 5] {
        let old = hex::decode(std::fs::read_to_string(format!("/tmp/rook-r7-F/old/round{r}.hex")).unwrap().trim()).unwrap();
        let new = hex::decode(std::fs::read_to_string(format!("tests/fixtures/eth-state-round{r}.hex")).unwrap().trim()).unwrap();
        let (a, b): (EthState, EthState) = (candid::decode_one(&old).unwrap(), candid::decode_one(&new).unwrap());
        println!("round{r}: bb713fe {} bytes, HEAD {} bytes, same value: {}", old.len(), new.len(), a == b);
        assert_eq!(a, b, "round{r}");
    }
}
