//! The canister's port of the floor module's pure rules (`autobuyer/canister-rs/src/solread.rs`: `floor_bounds`, `item_cap`,
//! `max_item_cost`, `net_units`, `arrival_room`, `fresh`) and of the wrapper's hashing (`frogbuy::leaf_of`, `verify_proof`, the
//! Inscriptions source `keccak(tx ‖ vout be)`) is held to THIS crate's own implementation by vectors computed here, from these
//! functions, and checked there (`canister-rs/tests/solana.rs`). Phase B2 closed B1's circularity: B1's port was checked only
//! against itself.
//!
//! `cargo test -p rook_core --lib vectors` checks the committed file against what these functions compute now (a change of a rule
//! here fails until the vectors are written again and the canister's test is run); `ROOK_WRITE_VECTORS=1` writes them.

use anchor_lang::AccountDeserialize;

use crate::hashing::{is_taproot, is_taproot_on, keccak, leaf_of, verify_proof};
use crate::state::floor::{Module, MAX_ITEMS_PER_DAY};
use crate::params::default_params;
use crate::units::{LAMPORTS_PER_SOL, ONE};

/// A revenue target for the vectors (the launch file's value at launch; none of these rules reads it).
const TEST_REVENUE_TARGET: u64 = 20 * LAMPORTS_PER_SOL;

const OUT: &str = "../../../autobuyer/canister-rs/tests/fixtures/solana/rook-core-vectors.json";

/// xorshift64*: deterministic, no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[self.below(v.len() as u64) as usize]
    }
}

fn zero_module() -> Module {
    let mut d = vec![0u8; 8 + <Module as anchor_lang::Space>::INIT_SPACE];
    d[..8].copy_from_slice(<Module as anchor_lang::Discriminator>::DISCRIMINATOR);
    Module::try_deserialize_unchecked(&mut &d[..]).expect("a zeroed module decodes")
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn floor_case(r: &mut Rng) -> String {
    let mut m = zero_module();
    let mut p = default_params(TEST_REVENUE_TARGET);
    p.max_move_bps = r.pick(&[1, 100, 500, 1_999, 2_000]);
    p.conv_bps = r.pick(&[0, 300, 1_250, 2_000]);
    p.max_items_per_day = r.pick(&[1, 3, 7, 32]);
    p.min_confidence_bps = r.pick(&[0, 3_000, 8_000]);
    p.buffer_bps = r.pick(&[0, 500, 2_000]);
    m.params = p;
    let now: i64 = 1_791_565_571 + r.below(86_400 * 3) as i64;
    let scale = r.pick(&[1_000u64, 1_000_000, 4_900_000_000, 255_000_000_000, 9_000_000_000_000]);
    m.accepted = if r.below(8) == 0 { 0 } else { 1 + r.below(scale) };
    m.posted_at = now - r.pick(&[0i64, 1, 59, 3_599, 3_600, 3_601, 10_799, 10_800, 86_400, -30]);
    m.confidence_bps = r.pick(&[0, 2_999, 3_000, 8_000, 10_000]);
    m.day_anchor = if r.below(4) == 0 { 0 } else { 1 + r.below(scale) };
    m.day_anchor_at = now - r.pick(&[0i64, 3_600, 86_399, 86_400, 90_000]);
    m.backing = r.below(scale.saturating_mul(4));
    m.revenue = r.below(scale / 3 + 1);
    m.surplus = r.below(scale / 5 + 1);
    m.transit_units = r.pick(&[0, ONE]);
    m.reserve_units = r.pick(&[0, ONE / 2, ONE, 3 * ONE]);
    let snap = 1 + r.below(scale.saturating_mul(2));
    m.snap_cap = r.pick(&[0, snap]);
    m.snap_at = now - r.pick(&[0i64, 100, 7_199, 7_200, 9_000]);
    m.arrivals = r.below(40);
    for i in 0..MAX_ITEMS_PER_DAY {
        m.arrived_at[i] = now - r.pick(&[0i64, 3_600, 86_399, 86_400, 90_000]);
    }
    let e = r.pick(&[0, ONE / 2, ONE, 2 * ONE, 5 * ONE + 7, 40 * ONE]);
    let kd = r.below(5) == 0;
    let (lo, hi) = m.floor_bounds(now).expect("bounds");
    let item_cap = m.item_cap(now, kd).expect("item cap");
    let mic = m.max_item_cost(now, e, kd).expect("max item cost");
    let arr: Vec<String> = m.arrived_at.iter().map(|x| x.to_string()).collect();
    format!(
        "{{\"now\":{now},\"e\":{e},\"kd\":{kd},\"max_move_bps\":{},\"conv_bps\":{},\"max_items_per_day\":{},\"min_confidence_bps\":{},\
\"buffer_bps\":{},\"accepted\":{},\"posted_at\":{},\"confidence_bps\":{},\"day_anchor\":{},\"day_anchor_at\":{},\"backing\":{},\"revenue\":{},\
\"surplus\":{},\"transit_units\":{},\"reserve_units\":{},\"snap_cap\":{},\"snap_at\":{},\"arrivals\":{},\"arrived_at\":[{}],\
\"out\":{{\"fresh\":{},\"bounds\":[{lo},{hi}],\"item_cap\":{item_cap},\"max_item_cost\":{mic},\"net_units\":{},\"arrival_room\":{}}}}}",
        p.max_move_bps, p.conv_bps, p.max_items_per_day, p.min_confidence_bps, p.buffer_bps, m.accepted, m.posted_at, m.confidence_bps,
        m.day_anchor, m.day_anchor_at, m.backing, m.revenue, m.surplus, m.transit_units, m.reserve_units, m.snap_cap, m.snap_at, m.arrivals,
        arr.join(","), m.fresh(now), m.net_units(e), m.arrival_room(now)
    )
}

fn hashing_cases() -> String {
    let ids = ["a", "0202020202020202020202020202020202020202020202020202020202020202i0",
        "6fb976ab49dcec017f1e201e84395983204ae1a7c2abf7ced0a85d692e442799i0", "b61b0172d95e266c18aea0c624db987e971a5d6d4ebc2aaed85da4642d635735i42",
        "e"];
    let leaves: Vec<[u8; 32]> = ids.iter().map(|i| leaf_of(i)).collect();
    let pair = |a: [u8; 32], b: [u8; 32]| if a <= b { keccak(&[&a, &b]) } else { keccak(&[&b, &a]) };
    // a five-leaf tree, OpenZeppelin's shape (the odd one carried up): ((0,1),(2,3)),4
    let (n01, n23) = (pair(leaves[0], leaves[1]), pair(leaves[2], leaves[3]));
    let n0123 = pair(n01, n23);
    let root = pair(n0123, leaves[4]);
    let proofs: Vec<Vec<[u8; 32]>> = vec![vec![leaves[1], n23, leaves[4]], vec![leaves[0], n23, leaves[4]], vec![leaves[3], n01, leaves[4]],
        vec![leaves[2], n01, leaves[4]], vec![n0123]];
    let mut out = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        assert!(verify_proof(&proofs[i], &root, leaves[i]));
        let pr: Vec<String> = proofs[i].iter().map(|p| format!("\"{}\"", hex(p))).collect();
        out.push(format!("{{\"id\":\"{id}\",\"leaf\":\"{}\",\"proof\":[{}],\"verifies\":true}}", hex(&leaves[i]), pr.join(",")));
    }
    // a proof of another leaf does not verify
    assert!(!verify_proof(&proofs[0], &root, leaves[1]));
    let wrong: Vec<String> = proofs[0].iter().map(|p| format!("\"{}\"", hex(p))).collect();
    out.push(format!("{{\"id\":\"{}\",\"leaf\":\"{}\",\"proof\":[{}],\"verifies\":false}}", ids[1], hex(&leaves[1]), wrong.join(",")));
    let sources: Vec<String> = [([7u8; 32], 0u32), ([0xab; 32], 1), ([0x01; 32], 0xdead_beef)].iter().map(|(tx, vout)| {
        format!("{{\"tx\":\"{}\",\"vout\":{vout},\"source\":\"{}\"}}", hex(tx), hex(&keccak(&[tx, &vout.to_be_bytes()])))
    }).collect();
    let taproots: Vec<String> = [crate::hashing::BTC_BURN_ADDRESS, "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr",
        "bc1p49upe77txy03ysp4m3r4m992tf27d2xgce7yk6w53qg5jtc85exqtkpg3t", "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq",
        "tb1p49upe77txy03ysp4m3r4m992tf27d2xgce7yk6w53qg5jtc85exqtkpg3t", "bc1p49upe77txy03ysp4m3r4m992tf27d2xgce7yk6w53qg5jtc85exqtkpg3q"]
        .iter().map(|a| format!("{{\"address\":\"{a}\",\"taproot\":{}}}", is_taproot(a))).collect();
    // 2026-10-09: the registry's Bitcoin network (`BitcoinNetwork`): the same addresses and two signet ones, checked as `tb`
    let tb: Vec<String> = [crate::hashing::BTC_BURN_ADDRESS, "tb1p2zffkaxp5py4fdutfdsrt6t6tcrc5ks09rkfd428hlhf4n5q8tqqeygjde",
        "tb1p49upe77txy03ysp4m3r4m992tf27d2xgce7yk6w53qg5jtc85exqu7h8ty", "tb1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqp3mvzv",
        "tb1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr", "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq",
        "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"]
        .iter().map(|a| format!("{{\"address\":\"{a}\",\"taproot\":{}}}", is_taproot_on(a, "tb"))).collect();
    format!("\"root\":\"{}\",\"leaves\":[{}],\"sources\":[{}],\"taproot\":[{}],\"taproot_tb\":[{}]", hex(&root), out.join(","), sources.join(","),
        taproots.join(","), tb.join(","))
}

fn vectors() -> String {
    let mut r = Rng(0x9e37_79b9_7f4a_7c15);
    let cases: Vec<String> = (0..400).map(|_| floor_case(&mut r)).collect();
    format!("{{\"generator\":\"solana/programs/rook_core/src/vectors.rs\",\"floor\":[\n{}\n],{}}}\n", cases.join(",\n"), hashing_cases())
}

#[test]
fn the_canisters_vectors_are_what_this_crate_computes() {
    let v = vectors();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(OUT);
    if std::env::var("ROOK_WRITE_VECTORS").is_ok() {
        std::fs::write(&path, &v).expect("write the vectors");
    }
    let committed = std::fs::read_to_string(&path).expect("the committed vectors (ROOK_WRITE_VECTORS=1 writes them)");
    assert_eq!(committed, v, "rook_core's rules changed: write the vectors again (ROOK_WRITE_VECTORS=1) and run the canister's tests");
}
