//! Integration R3 (docs/audit/2026-09-26/review2/integration-R3.md §1): the calls the custodian makes on the Wrapper are the
//! committed contract's, not a working tree's. `markUndeliverable` was coded against r3-contracts' design; these tests hold the
//! canister's bytes to `cast calldata` (the fixture `test/fixtures/canister/mark-undeliverable-7.hex`, which
//! `test/audit2/R3CanisterCall.t.sol` holds to `abi.encodeCall(Wrapper.markUndeliverable, (7))` and runs from the custodian), and,
//! when the contracts are built (`forge build`), every Wrapper selector the canister uses and the `ticket(uint256)` struct it
//! decodes to Foundry's own artifact.

use etch_autobuyer::redeem;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../");

fn fixture(rel: &str) -> Vec<u8> {
    let s = std::fs::read_to_string(format!("{ROOT}{rel}")).expect("fixture");
    hex::decode(s.trim().trim_start_matches("0x")).expect("hex")
}

#[test]
fn mark_undeliverable_is_the_committed_wrappers_calldata() {
    // `cast calldata "markUndeliverable(uint256)" 7`, and the forge test encodes the same with the contract's own ABI
    assert_eq!(redeem::mark_undeliverable(7), fixture("test/fixtures/canister/mark-undeliverable-7.hex"));
    assert_eq!(hex::encode(&redeem::mark_undeliverable(0)[..4]), "acf756cc");
    // one uint256 argument, nothing else: a large id is one word, big-endian
    let c = redeem::mark_undeliverable(u64::MAX);
    assert_eq!(c.len(), 4 + 32);
    assert_eq!(&c[4..28], &[0u8; 24]);
    assert_eq!(&c[28..], &u64::MAX.to_be_bytes());
}

#[test]
fn every_wrapper_selector_the_canister_uses_is_in_foundrys_artifact() {
    let path = format!("{ROOT}out/Wrapper.sol/Wrapper.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("skipped: {path} not built (run `forge build` at the repo root); the calldata fixture test still ran");
        return;
    };
    let art: serde_json::Value = serde_json::from_str(&text).expect("artifact json");
    let ids = &art["methodIdentifiers"];
    for (sig, sel) in [
        ("markUndeliverable(uint256)", &redeem::mark_undeliverable(0)[..4]),
        ("recordDelivery(uint256,bytes32)", &redeem::RECORD_DELIVERY[..]),
        ("ticket(uint256)", &redeem::TICKET[..]),
        ("ticketCount()", &redeem::TICKET_COUNT[..]),
        ("custodian()", &redeem::CUSTODIAN[..]),
        ("registry()", &redeem::REGISTRY[..]),
        ("listingId()", &redeem::LISTING_ID[..]),
    ] {
        assert_eq!(ids[sig].as_str(), Some(hex::encode(sel).as_str()), "{sig}");
    }
    let abi = art["abi"].as_array().expect("abi");
    let f = |name: &str| abi.iter().find(|e| e["type"] == "function" && e["name"] == name).expect(name).clone();
    let m = f("markUndeliverable");
    assert_eq!(m["inputs"].as_array().unwrap().len(), 1);
    assert_eq!(m["inputs"][0]["type"], "uint256");
    assert_eq!(m["stateMutability"], "nonpayable");
    // `decode_ticket` reads stage at word 0, deliverBy 5, deliverByBlock 6, acquisitionId 7, destination 9
    let comps: Vec<String> = f("ticket")["outputs"][0]["components"].as_array().unwrap().iter()
        .map(|c| c["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(comps, ["stage", "owner", "round", "pinnedAt", "pinnedAtBlock", "deliverBy", "deliverByBlock", "acquisitionId",
        "deliveryTxid", "destination"]);
    // the stages decode_ticket maps (Resolved = 2 is the one markUndeliverable accepts)
    let src = std::fs::read_to_string(format!("{ROOT}src/Wrapper.sol")).expect("Wrapper.sol");
    assert!(src.contains("enum Stage { None, Pinned, Resolved, Delivered, Refunded }"));
    let body = src.split("function markUndeliverable(uint256 id) external {").nth(1).expect("markUndeliverable in Wrapper.sol");
    let body = &body[..body.find('}').unwrap()];
    assert!(body.contains("msg.sender != custodian") && body.contains("Stage.Resolved"), "custodian-only, Resolved only: {body}");
}

/// Fix round 3 (D3-H-1): every EthNftWrapper selector the canister's Ethereum side calls or reads, `noteSent` included, is the built
/// contract's (skipped until `forge build`), and `refund` refuses the late window once a delivery was noted sent.
#[test]
fn every_eth_nft_wrapper_selector_the_canister_uses_is_in_foundrys_artifact() {
    use etch_autobuyer::ethnft as en;
    let path = format!("{ROOT}out/EthNftWrapper.sol/EthNftWrapper.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("skipped: {path} not built (run `forge build` at the repo root)");
        return;
    };
    let art: serde_json::Value = serde_json::from_str(&text).expect("artifact json");
    let ids = &art["methodIdentifiers"];
    for (sig, sel) in [
        ("ticketCount()", en::TICKET_COUNT), ("ticket(uint256)", en::TICKET), ("item(uint256)", en::ITEM),
        ("mint(uint256,bytes32,address)", en::MINT), ("mintDeposit(uint256,bytes32,address)", en::MINT_DEPOSIT),
        ("recordDelivery(uint256,bytes32)", en::RECORD_DELIVERY), ("markUndeliverable(uint256)", en::MARK_UNDELIVERABLE),
        ("noteSent(uint256,bytes32)", en::NOTE_SENT), ("itemOf(uint256)", en::ITEM_OF), ("chainId()", en::CHAIN_ID),
        ("collection()", en::COLLECTION), ("custodian()", en::CUSTODIAN),
        // review round 5 (F5-M-1): what noteSent and markUndeliverable leave, read back before either is taken as not having happened
        ("sentTx(uint256)", en::SENT_TX), ("undeliverable(uint256)", en::UNDELIVERABLE),
    ] {
        assert_eq!(ids[sig].as_str(), Some(hex::encode(sel).as_str()), "{sig}");
    }
    assert_eq!(en::note_sent_call(7, [9; 32]), [&en::NOTE_SENT[..], &[0u8; 31], &[7], &[9u8; 32]].concat());
    let src = std::fs::read_to_string(format!("{ROOT}src/nft/EthNftWrapper.sol")).expect("EthNftWrapper.sol");
    let body = src.split("function refund(uint256 id) external {").nth(1).expect("refund");
    assert!(body.contains("sentTx[id] != bytes32(0)") && body.contains("DeliverySentNotRecorded"), "the late window refuses a noted delivery");
}
