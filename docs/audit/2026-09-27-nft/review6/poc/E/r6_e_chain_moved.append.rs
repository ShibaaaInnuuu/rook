
// Review round 6, part E (E6-I-2): F5-L-1's reset looks only at the saved and the new chain of ONE upgrade. Dropping eth_nft (allowed
// when nothing is open) and adding it back on another chain in a later upgrade keeps chain 1's floors: every pin of the new chain below
// chain 1's floor is refused. Passes while the weakness is present.
#[test]
fn r6_e6_i2_a_two_step_chain_move_keeps_the_old_chains_floors() {
    let mut fl = etch_autobuyer::pin::PinFloors::default();
    fl.eth.admit_pin("Ethereum", ETH_PIN).unwrap();
    fl.eth.admit_nonce("Ethereum", 40).unwrap();
    let mut st = EthState::default();
    st.last_mined = Some((ETH_PIN, 40));
    // upgrade 1: eth_nft dropped
    assert!(!ef::chain_moved(&mut st, &mut fl, Some(1), None));
    // upgrade 2: eth_nft back, on chain 8453 (from = None: the saved Init has no eth_nft)
    assert!(!ef::chain_moved(&mut st, &mut fl, None, Some(8453)), "no reset");
    assert!(st.last_mined.is_some());
    assert!(fl.eth.admit_pin("chain 8453", ETH_PIN - 1_000).is_err(), "a lower head on the new chain is refused");
    assert!(fl.eth.admit_nonce("chain 8453", 0).is_err(), "the new chain's first nonce is refused");
}
