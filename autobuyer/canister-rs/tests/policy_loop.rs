//! The purchase policy through the loop (`docs/v2/venue-bridge-policy.md` §1): a fake vault and registry answer each read with
//! real ABI bytes, and `looper::maybe_open` must open exactly what the chain says is owed — or wait, and say why.

use etch_autobuyer::bridge::{BridgeQuote, Fixture as BridgeFixture};
use etch_autobuyer::evmtx::VaultPurchase;
use etch_autobuyer::looper::{maybe_open, BtcView, EvmView, Io};
use etch_autobuyer::policy::{self, MAX_PURCHASE_WEI};
use etch_autobuyer::psbt::Quote;
use etch_autobuyer::runestone::RuneId;
use etch_autobuyer::state::{Phase, Purchase};
use etch_autobuyer::venue::Fixture as VenueFixture;
use std::collections::HashMap;

const VAULT: [u8; 20] = [1; 20];
const REGISTRY: [u8; 20] = [2; 20];

fn word(v: u128) -> Vec<u8> {
    let mut w = vec![0u8; 16];
    w.extend_from_slice(&v.to_be_bytes());
    w
}
fn addr(a: [u8; 20]) -> Vec<u8> {
    [vec![0u8; 12], a.to_vec()].concat()
}
/// `registry.listing(id)` as the registry returns it: `cast abi-encode` of a rune listing with divisibility 2, ref "840000:3"
/// and the BIP 86 address, with its vault field set to `vault`.
fn listing(vault: [u8; 20], rune: bool, paused: bool) -> Vec<u8> {
    let mut h = hex::decode(concat!(
        "0000000000000000000000000000000000000000000000000000000000000020",
        "0000000000000000000000000000000000000000000000000000000000000001",
        "0000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000001",
        "0000000000000000000000000000000000000000000000000000000000000002",
        "0000000000000000000000000101010101010101010101010101010101010101",
        "0000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000120",
        "0000000000000000000000000000000000000000000000000000000000000160",
        "00000000000000000000000000000000000000000000000000000000000001a0",
        "0000000000000000000000000000000000000000000000000000000000000004",
        "4d4f4f4e00000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000008",
        "3834303030303a33000000000000000000000000000000000000000000000000",
        "000000000000000000000000000000000000000000000000000000000000003e",
        "62633170356379786e75786d65757775766b7766656d39366c717a737a643032",
        "6e367864636a727332306361633679716a6a7775647078716b65647263720000",
    )).unwrap();
    h[32 + 31] = rune as u8;
    h[64 + 31] = paused as u8;
    h[160 + 12..192].copy_from_slice(&vault);
    h
}

struct World {
    now: u64,
    reads: HashMap<([u8; 20], Vec<u8>), Vec<u8>>,
    payers: HashMap<u128, u64>,
    count: u64,
    configured: bool,
    cost: Option<u128>,
    venue: VenueFixture,
    bridge: BridgeFixture,
}

impl World {
    fn new() -> Self {
        let mut w = World {
            now: 1_000_000,
            reads: HashMap::new(),
            payers: HashMap::new(),
            count: 4,
            configured: true,
            cost: Some(1_000_000_000_000_000),
            venue: VenueFixture { quote: Quote { rune: RuneId { block: 1, tx: 0 }, units: 1, max_spend_sats: 0, max_fee_sats: 0,
                recipient_script: vec![], our_script: vec![] }, psbt: vec![], to_sign: vec![] },
            bridge: BridgeFixture { quote: BridgeQuote { send_wei: 0, expect_sats: 0, to: [0; 20], calldata: vec![], recipient_script: vec![] } },
        };
        w.set(VAULT, policy::call0(policy::REGISTRY), addr(REGISTRY));
        w.set(VAULT, policy::call0(policy::LISTING_ID), word(0));
        w.set(REGISTRY, policy::call_uint(policy::LISTING, 0), listing(VAULT, true, false));
        w.set(VAULT, policy::call0(policy::ATTRIBUTED), word(300_000_000_000_000_000));
        w.set(VAULT, policy::call0(policy::CAP_WEI), word(1_000_000_000_000_000));
        w.set(VAULT, policy::call0(policy::CAP_EXPIRY), word(1_000_000 + 3_600));
        w.set(VAULT, policy::call0(policy::MAX_IN_FLIGHT), word(1_000_000_000_000_000_000));
        w.set(VAULT, policy::call0(policy::IN_FLIGHT), word(0));
        w.set(VAULT, policy::call0(policy::OPEN_PURCHASES), [word(32), word(0)].concat());
        w.set(VAULT, policy::call0(policy::IS_BLOCKED), [word(0), word(0)].concat());
        w.payers.insert(300_000_000_000_000_000, 3);
        w
    }
    fn set(&mut self, to: [u8; 20], data: Vec<u8>, ret: Vec<u8>) {
        self.reads.insert((to, data), ret);
    }
}

impl Io for World {
    fn now(&self) -> u64 { self.now }
    fn chain_id(&self) -> u64 { 31337 }
    fn vault(&self) -> [u8; 20] { VAULT }
    async fn our_script(&mut self) -> Result<Vec<u8>, String> { unimplemented!() }
    async fn purchase_count(&mut self) -> Result<u64, String> { Ok(self.count) }
    async fn vault_purchase(&mut self, _id: u64) -> Result<VaultPurchase, String> { unimplemented!() }
    async fn evm_balance(&mut self) -> Result<u128, String> { unimplemented!() }
    async fn evm_nonce(&mut self) -> Result<u64, String> { unimplemented!() }
    async fn base_fee(&mut self) -> Result<u128, String> { unimplemented!() }
    async fn estimate_gas(&mut self, _to: [u8; 20], _value: u128, _data: &[u8]) -> Result<u64, String> { unimplemented!() }
    async fn sign_evm(&mut self, _hash: [u8; 32]) -> Result<(u8, [u8; 32], [u8; 32]), String> { unimplemented!() }
    async fn send_evm(&mut self, _raw: &[u8]) -> Result<(), String> { unimplemented!() }
    async fn evm_seen(&mut self, _hash: [u8; 32]) -> EvmView { unimplemented!() }
    fn save(&mut self, _p: &Purchase) {}
    async fn btc_sats(&mut self) -> Result<u64, String> { unimplemented!() }
    async fn btc_seen(&mut self, _txid: [u8; 32]) -> BtcView { unimplemented!() }
    async fn btc_send(&mut self, _raw: &[u8]) -> Result<(), String> { unimplemented!() }
    async fn delivered_units(&mut self, _txid: [u8; 32], _quote: &Quote) -> Result<u128, String> { unimplemented!() }
    async fn sign_taproot(&mut self, _sighash: [u8; 32]) -> Result<Vec<u8>, String> { unimplemented!() }
    async fn input_clean(&mut self, _o: etch_autobuyer::custody::Outpoint) -> Result<(), String> { unimplemented!() }
    fn venue(&self) -> Option<&dyn etch_autobuyer::venue::Venue> { self.configured.then_some(&self.venue as _) }
    fn bridge(&self) -> Option<&dyn etch_autobuyer::bridge::Bridge> { self.configured.then_some(&self.bridge as _) }
    fn bridge_allowlist(&self) -> Vec<[u8; 20]> { vec![] }
    async fn reference_floor_sats(&mut self, _send_wei: u128) -> Result<u64, String> { unimplemented!() }
    async fn reference_cap_sats(&mut self, _wei: u128) -> Result<u64, String> { unimplemented!() }
    async fn eth_read(&mut self, to: [u8; 20], data: &[u8]) -> Result<Vec<u8>, String> {
        if data.starts_with(&policy::PAYERS_FOR) {
            let a = u128::from_be_bytes(data[4 + 16..36].try_into().unwrap());
            return Ok(word(*self.payers.get(&a).unwrap_or(&0) as u128));
        }
        self.reads.get(&(to, data.to_vec())).cloned().ok_or_else(|| format!("no read for {}", hex::encode(data)))
    }
    async fn route_cost_wei(&mut self) -> Result<u128, String> { self.cost.ok_or("no route cost estimate".into()) }
    fn btc_hrp(&self) -> Result<&'static str, String> { Ok("bc") }
}

fn run<F: std::future::Future>(f: F) -> F::Output {
    // every await in the fake is ready at once
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
    fn noop(_: *const ()) {}
    fn clone(p: *const ()) -> RawWaker { RawWaker::new(p, &VT) }
    static VT: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
    let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VT)) };
    let mut f = std::pin::pin!(f);
    match f.as_mut().poll(&mut Context::from_waker(&waker)) {
        Poll::Ready(x) => x,
        Poll::Pending => panic!("the fake world never waits"),
    }
}

#[test]
fn the_policy_opens_what_the_vault_says_is_owed_at_the_vaults_next_id() {
    let mut w = World::new();
    let p = run(maybe_open(&mut w, &[])).unwrap().expect("opens");
    assert_eq!((p.id, p.amount_wei, p.phase.clone()), (4, 300_000_000_000_000_000, Phase::Watching));
    let o = p.order.unwrap();
    assert_eq!((o.rune.as_str(), o.units, o.cap_ref.as_str()), ("840000:3", 30_000, "cap:1000000000000000@1003600"));
    assert_eq!(hex::encode(o.recipient_script), "5120a60869f0dbcf1dc659c9cecbaf8050135ea9e8cdc487053f1dc6880949dc684c");
}

#[test]
fn too_many_payers_halves_the_amount_until_the_vault_can_charge_it() {
    let mut w = World::new();
    w.payers.insert(300_000_000_000_000_000, 70);
    w.payers.insert(150_000_000_000_000_000, 5);
    assert_eq!(run(maybe_open(&mut w, &[])).unwrap().unwrap().amount_wei, 150_000_000_000_000_000);
    w.payers.insert(150_000_000_000_000_000, 0);
    assert!(run(maybe_open(&mut w, &[])).unwrap_err().contains("chargeable"), "nothing fundable: waits");
}

#[test]
fn it_opens_nothing_while_a_purchase_runs_without_a_route_or_for_another_vaults_listing() {
    let mut w = World::new();
    let running = Purchase { id: 3, phase: Phase::Funded, amount_wei: 1, order: None, pending_evm: None };
    assert_eq!(run(maybe_open(&mut w, &[running])).unwrap(), None, "one purchase at a time");
    w.configured = false;
    assert!(run(maybe_open(&mut w, &[])).unwrap_err().contains("configured"));
    let mut w = World::new();
    w.cost = None;
    assert!(run(maybe_open(&mut w, &[])).unwrap_err().contains("route cost"));
    let mut w = World::new();
    w.set(REGISTRY, policy::call_uint(policy::LISTING, 0), listing([9; 20], true, false));
    assert!(run(maybe_open(&mut w, &[])).unwrap_err().contains("not ours"));
    let mut w = World::new();
    w.set(REGISTRY, policy::call_uint(policy::LISTING, 0), listing(VAULT, true, true));
    assert!(run(maybe_open(&mut w, &[])).unwrap_err().contains("paused"));
    let mut w = World::new();
    w.set(VAULT, policy::call0(policy::ATTRIBUTED), word(10 * MAX_PURCHASE_WEI));
    assert_eq!(run(maybe_open(&mut w, &[])).map(|p| p.map(|p| p.amount_wei)), Err("no amount from [500000000000000000, 250000000000000000, 125000000000000000, 62500000000000000] is chargeable within 64 payers ([0, 0, 0, 0])".into()),
        "a big vault is capped at 0.5 ETH, and payersFor must answer for that amount");
}
