
// Review round 6, part E: the round-2 and round-4 fixtures as 8ef82d6 held them (before this round re-encoded them) decode in the HEAD
// build with nothing lost and re-encode to the committed bytes' values.
#[test]
fn r6_e_the_pre_round5_fixture_bytes_decode_losslessly() {
    use etch_autobuyer::canister::lost_opts;
    for (name, old, new) in [("round2", include_str!("fixtures/old-round2.hex"), include_str!("fixtures/eth-state-round2.hex")),
        ("round4", include_str!("fixtures/old-round4.hex"), include_str!("fixtures/eth-state-round4.hex"))] {
        let ob = hex::decode(old.trim()).unwrap();
        let e: EthState = candid::decode_one(&ob).unwrap();
        let again = candid::encode_one(&e).unwrap();
        assert_eq!(lost_opts(&ob, &again).unwrap(), Vec::<String>::new(), "{name}");
        let n: EthState = candid::decode_one(&hex::decode(new.trim()).unwrap()).unwrap();
        assert_eq!(e, n, "{name}: the re-encoded fixture holds the same state");
        println!("{name}: old {} bytes, new {} bytes, same value", ob.len(), new.trim().len() / 2);
    }
}
