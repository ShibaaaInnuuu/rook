//! Prints the candid encoding of the buyer's install argument, as hex, for a timelock proposal: an upgrade's argument is bytes,
//! and the timelock publishes them. The struct mirrors `etch_autobuyer::canister::Init`; candid matches records by field name.
//!
//! cargo run --example encode_init -- <operator-principal>
use candid::{CandidType, Encode, Principal};

#[derive(CandidType)]
struct Init {
    chain_id: u64,
    vault: String,
    rpc_urls: Vec<String>,
    btc_api_urls: Vec<String>,
    key_name: String,
    accept_production_key_name: bool,
    bridge_allowlist: Vec<String>,
    btc_network: String,
    operator: Option<Principal>,
}

fn main() {
    let operator = std::env::args().nth(1).map(|p| Principal::from_text(p).expect("a principal"));
    let init = Init {
        chain_id: 46630,
        vault: "0x0000000000000000000000000000000000000001".into(),
        rpc_urls: vec!["https://rpc.testnet.chain.robinhood.com".into(), "https://explorer.testnet.chain.robinhood.com/api/eth-rpc".into()],
        btc_api_urls: vec!["https://mempool.space/signet/api".into(), "https://mutinynet.com/api".into()],
        key_name: "key_1".into(),
        accept_production_key_name: true,
        bridge_allowlist: vec![],
        btc_network: "signet".into(),
        operator,
    };
    println!("{}", hex::encode(Encode!(&init).expect("encodes")));
}
