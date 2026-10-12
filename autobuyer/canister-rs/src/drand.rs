//! drand for the registry keeper (decision D-i, docs/v9/solana-migration.md §2.9): the randomness of a redemption ticket's pinned
//! round, fetched from independent public drand HTTP relays of the chain rook_core uses, believed only when at least two relays give
//! EXACTLY the same signature and randomness and no relay gives anything else, then written by the canister's key with `prove_round`.
//!
//! The chain is drand's **evmnet** (rook_core `clock.rs`: genesis 1,727,521,075, a round every 3 s, as `src/rand/DrandEvmnet.sol`): its
//! signatures are BLS on BN254 (G1, 64 bytes), and a round's randomness is `sha256(signature)` (drand's own `randomness` field and
//! `DrandEvmnet.verify`'s return), which every relay answer must satisfy. A round is never asked before it exists (`round_at(now)`;
//! rook_core refuses a future round anyway, audit A2-02).
//!
//! The trust (Q4 still open): rook_core's `prove_round` is keeper-attested until the BN254 pairing check is ported to the alt_bn128
//! syscalls, so the randomness is what the canister writes. The canister does not verify the BLS signature itself (see the report in
//! docs/v9/solana-migration.md §2.9 D-i: the pairing would cost the wasm size budget); the rule is the agreement of independent relays
//! (two of the League of Entropy's own and Cloudflare's, operated apart), and the signature is kept in the canister's log so anyone can
//! check it against the evmnet public key afterwards.
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// drand evmnet's chain hash (`keeper/service/drand.ts`, measured 2026-09-26 on every relay below).
pub const EVMNET_CHAIN: &str = "04f1e9062b8a81f848fded9c12306733282b2727ecced50032187751166ec8c3";
/// rook_core `clock.rs` `DRAND_GENESIS` / `DRAND_PERIOD` (evmnet).
pub const GENESIS: u64 = 1_727_521_075;
pub const PERIOD: u64 = 3;
/// The relays asked when the install names none (`Home::Solana.drand_urls`): the League of Entropy's own and Cloudflare's.
pub const DEFAULT_RELAYS: [&str; 4] = ["https://api.drand.sh", "https://api2.drand.sh", "https://api3.drand.sh", "https://drand.cloudflare.com"];
/// Relays that must give the same bytes.
pub const NEED: usize = 2;
/// A beacon's JSON is ≈ 300 bytes; the cap is paid for, used or not.
pub const MAX_BYTES: u64 = 2_000;

/// `DrandEvmnet.roundAt`: the round live at unix time `t` (0 before genesis).
pub fn round_at(t: u64) -> u64 {
    if t < GENESIS { 0 } else { (t - GENESIS) / PERIOD + 1 }
}

/// One round's beacon as a relay serves it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Beacon {
    pub round: u64,
    /// The BLS signature (G1 on BN254, 64 bytes).
    pub signature: Vec<u8>,
    /// `sha256(signature)`.
    pub randomness: [u8; 32],
}

/// The relay's URL for `round` (drand's v1 path, the one every relay above serves for a chain hash).
pub fn url_of(base: &str, round: u64) -> String {
    format!("{}/{EVMNET_CHAIN}/public/{round}", base.trim_end_matches('/'))
}

/// The transform body (pure): what every replica must agree on. `{"beacon": {round, signature, randomness}}`, `{"none": status}` for a
/// round the relay does not have (404, 425, 400: not produced yet, or an unknown chain), `{"garbled": …}` for anything else.
pub fn normalise(status: u32, body: &[u8]) -> Value {
    if matches!(status, 400 | 404 | 425) {
        return json!({ "none": status });
    }
    if status != 200 {
        return json!({ "http": status });
    }
    let Ok(v) = serde_json::from_slice::<Value>(body) else { return json!({ "garbled": "unparsable" }) };
    match (v["round"].as_u64(), v["signature"].as_str(), v["randomness"].as_str()) {
        (Some(r), Some(s), Some(x)) => json!({ "beacon": { "round": r, "signature": s.to_lowercase(), "randomness": x.to_lowercase() } }),
        _ => json!({ "garbled": "fields" }),
    }
}

/// A transformed answer as a beacon: `Ok(None)` the relay does not have the round; `Err` anything that is not a well-formed beacon of
/// THIS round whose randomness is the sha256 of its signature (a relay serving that is wrong, and counts against agreement).
pub fn parse(round: u64, v: &Value) -> Result<Option<Beacon>, String> {
    if v.get("none").is_some() {
        return Ok(None);
    }
    let b = v.get("beacon").ok_or_else(|| format!("not a beacon: {v}"))?;
    let r = b["round"].as_u64().ok_or("no round")?;
    if r != round {
        return Err(format!("round {r} served for round {round}"));
    }
    let signature = hex::decode(b["signature"].as_str().unwrap_or("")).map_err(|e| format!("signature: {e}"))?;
    if signature.len() != 64 {
        return Err(format!("a signature of {} bytes (evmnet's are 64)", signature.len()));
    }
    let randomness: [u8; 32] = hex::decode(b["randomness"].as_str().unwrap_or("")).ok().and_then(|x| x.try_into().ok())
        .ok_or("randomness is not 32 bytes of hex")?;
    if <[u8; 32]>::from(Sha256::digest(&signature)) != randomness {
        return Err("its randomness is not sha256(signature)".into());
    }
    Ok(Some(Beacon { round, signature, randomness }))
}

/// What the relays say about one round.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// At least [`NEED`] relays gave exactly these bytes and none gave others.
    Proven { beacon: Beacon, relays: Vec<String> },
    /// Fewer than [`NEED`] relays have it yet (the others: not yet, or unreachable).
    NotYet { have: usize },
    /// Two relays gave different bytes, or one gave a malformed beacon: refused (an alert), nothing is proven.
    Disagree(String),
}

/// The agreement rule (pure): every relay's answer for `round`, by relay host. A relay that is unreachable or does not have the round
/// is not a veto; a relay that serves a beacon different from another's (or a malformed one) IS: nothing is proven while it does.
pub fn judge(round: u64, answers: &[(String, Result<Value, String>)], need: usize) -> Verdict {
    let mut got: Vec<(Beacon, Vec<String>)> = Vec::new();
    let mut bad = Vec::new();
    for (host, a) in answers {
        let Ok(v) = a else { continue };
        match parse(round, v) {
            Ok(None) => {}
            Ok(Some(b)) => match got.iter_mut().find(|(x, _)| *x == b) {
                Some((_, hs)) => hs.push(host.clone()),
                None => got.push((b, vec![host.clone()])),
            },
            Err(e) if v.get("http").is_some() => {
                let _ = e; // an HTTP error is an outage, not a lie
            }
            Err(e) => bad.push(format!("{host}: {e}")),
        }
    }
    if got.len() > 1 || !bad.is_empty() {
        let served: Vec<String> = got.iter().map(|(b, hs)| format!("{} = {}…", hs.join("+"), &hex::encode(&b.signature)[..16])).collect();
        return Verdict::Disagree(format!("drand round {round}: the relays disagree ({}{}{})", served.join(", "),
            if !served.is_empty() && !bad.is_empty() { "; " } else { "" }, bad.join("; ")));
    }
    match got.pop() {
        // the relays agree; the signature must also be evmnet's (the BLS check: neither the relays nor this canister's keeper role can
        // supply other randomness)
        Some((beacon, relays)) if relays.len() >= need => match verify(round, &beacon.signature) {
            Ok(_) => Verdict::Proven { beacon, relays },
            Err(e) => Verdict::Disagree(format!("drand round {round}: {} served a signature that does not verify: {e}", relays.join("+"))),
        },
        Some((_, relays)) => Verdict::NotYet { have: relays.len() },
        None => Verdict::NotYet { have: 0 },
    }
}

// ---- the BLS check (drand evmnet: `bls-bn254-unchained-on-g1`, as `src/rand/DrandEvmnet.sol` verifies it) --------------------------

/// `DrandEvmnet.DST`.
pub const DST: &[u8] = b"BLS_SIG_BN254G1_XMD:KECCAK-256_SVDW_RO_NUL_";
/// evmnet's public key (G2): x = (c0, c1), y = (c0, c1), big-endian (`DrandEvmnet.PK_*`).
const PK: [&str; 4] = [
    "0557ec32c2ad488e4d4f6008f89a346f18492092ccc0d594610de2732c8b808f",
    "07e1d1d335df83fa98462005690372c643340060d205306a9aa8106b6bd0b382",
    "297d3a4f9749b33eb2d904c9d9ebf17224150ddd7abd7567a9bec6c74480ee0b",
    "0095685ae3a85ba243747b1b2f426049010f6b73a0cf1d389351d5aaaa1047f6",
];
/// The SvdW constants for A = 0, B = 3, Z = 1 (`BN254HashToCurve.C2..C4`; C1 = 4).
const C2: &str = "183227397098d014dc2822db40c0ac2ecbc0b548b438e5469e10460b6c3e7ea3";
const C3: &str = "016789af3a83522eb353c98fc6b36d713d5d8d1cc5dffffffa";
const C4: &str = "10216f7ba065e00de81ac1e7808072c9dd2b2385cd7b438469602eb24829a9bd";

mod bls {
    use ark_bn254::{Bn254, Fq, Fq12, Fq2, G1Affine, G1Projective, G2Affine};
    use ark_ec::{pairing::Pairing, AffineRepr, CurveGroup};
    use ark_ff::{BigInteger, Field, One, PrimeField, Zero};

    fn fq(hex_be: &str) -> Fq {
        Fq::from_be_bytes_mod_order(&hex::decode(hex_be).expect("a constant"))
    }

    fn keccak(parts: &[&[u8]]) -> [u8; 32] {
        use sha3::{Digest, Keccak256};
        let mut h = Keccak256::new();
        for p in parts {
            h.update(p);
        }
        h.finalize().into()
    }

    /// RFC 9380 §5.3.1 `expand_message_xmd` with Keccak-256 (b = 32, s = 136), `len` ≤ 255 · 32.
    pub fn expand_message_xmd(dst: &[u8], msg: &[u8], len: u16) -> Vec<u8> {
        let ell = (len as usize).div_ceil(32);
        let mut dst_prime = dst.to_vec();
        dst_prime.push(dst.len() as u8);
        let b0 = keccak(&[&[0u8; 136], msg, &len.to_be_bytes(), &[0], &dst_prime]);
        let mut bi = keccak(&[&b0, &[1], &dst_prime]);
        let mut out = bi.to_vec();
        for i in 2..=ell {
            let x: Vec<u8> = b0.iter().zip(bi.iter()).map(|(a, b)| a ^ b).collect();
            bi = keccak(&[&x, &[i as u8], &dst_prime]);
            out.extend_from_slice(&bi);
        }
        out.truncate(len as usize);
        out
    }

    fn pow(x: Fq, e: &ark_ff::BigInt<4>) -> Fq {
        x.pow(e.as_ref())
    }

    fn sqrt_exp() -> ark_ff::BigInt<4> {
        // (P + 1) / 4
        let mut p = Fq::MODULUS;
        p.add_with_carry(&ark_ff::BigInt::from(1u64));
        p.div2();
        p.div2();
        p
    }

    fn is_square(a: Fq) -> bool {
        // Euler's criterion (P − 1) / 2
        let mut e = Fq::MODULUS;
        e.sub_with_borrow(&ark_ff::BigInt::from(1u64));
        e.div2();
        let l = pow(a, &e);
        l.is_zero() || l.is_one()
    }

    fn g(x: Fq) -> Fq {
        x * x * x + Fq::from(3u64)
    }

    fn parity(x: Fq) -> bool {
        x.into_bigint().is_odd()
    }

    /// The SvdW map, RFC 9380 §F.1, as `BN254HashToCurve.mapToPoint`.
    pub fn map_to_point(u: Fq) -> G1Affine {
        let (c1, c2, c3, c4) = (Fq::from(4u64), fq(super::C2), fq(super::C3), fq(super::C4));
        let mut tv1 = u * u * c1;
        let tv2 = Fq::one() + tv1;
        tv1 = Fq::one() - tv1;
        let tv3 = (tv1 * tv2).inverse().unwrap_or(Fq::zero());
        let tv4 = u * tv1 * tv3 * c3;
        let x1 = c2 - tv4;
        let x2 = c2 + tv4;
        let mut x3 = tv2 * tv2 * tv3;
        x3 = x3 * x3 * c4 + Fq::one();
        let x = if is_square(g(x1)) { x1 } else if is_square(g(x2)) { x2 } else { x3 };
        let mut y = pow(g(x), &sqrt_exp());
        if parity(u) != parity(y) {
            y = -y;
        }
        G1Affine::new_unchecked(x, y)
    }

    /// `hash_to_curve` (count 2, L 48) of `msg`: the message point.
    pub fn hash_to_point(dst: &[u8], msg: &[u8]) -> G1Affine {
        let u = expand_message_xmd(dst, msg, 96);
        let (u0, u1) = (Fq::from_be_bytes_mod_order(&u[..48]), Fq::from_be_bytes_mod_order(&u[48..]));
        (G1Projective::from(map_to_point(u0)) + map_to_point(u1)).into_affine()
    }

    /// `DrandEvmnet.verify`: the 64-byte G1 signature of `round` under evmnet's key.
    pub fn verify(round: u64, sig: &[u8]) -> Result<(), String> {
        if sig.len() != 64 {
            return Err("a signature of other than 64 bytes".into());
        }
        let coord = |b: &[u8]| -> Result<Fq, String> {
            let x = Fq::from_be_bytes_mod_order(b);
            // canonical: under the modulus (from_be_bytes_mod_order would reduce silently)
            if x.into_bigint().to_bytes_be() != b {
                return Err("a coordinate at or over the field's modulus".to_string());
            }
            Ok(x)
        };
        let (x, y) = (coord(&sig[..32])?, coord(&sig[32..])?);
        if x.is_zero() && y.is_zero() {
            return Err("the point at infinity".into());
        }
        let s = G1Affine::new_unchecked(x, y);
        if !s.is_on_curve() {
            return Err("the signature is not on the curve".into());
        }
        let h = hash_to_point(super::DST, &keccak(&[&round.to_be_bytes()]));
        let pk = G2Affine::new_unchecked(Fq2::new(fq(super::PK[0]), fq(super::PK[1])), Fq2::new(fq(super::PK[2]), fq(super::PK[3])));
        if !pk.is_on_curve() {
            return Err("evmnet's public key constant is not on the curve".into());
        }
        // e(−sig, G2) · e(H(m), pk) == 1
        let r = Bn254::multi_pairing([-s, h], [G2Affine::generator(), pk]);
        if r.0 != Fq12::one() {
            return Err(format!("the signature does not verify under evmnet's key for round {round}"));
        }
        Ok(())
    }
}

/// `DrandEvmnet.verify` in the canister: `signature` is evmnet's BLS signature of `round` (so its randomness, `sha256(signature)`, is
/// the round's, whoever served it). Pure; ≈ one pairing product of two.
pub fn verify(round: u64, signature: &[u8]) -> Result<[u8; 32], String> {
    bls::verify(round, signature)?;
    Ok(Sha256::digest(signature).into())
}

/// The relays an install may name: `https://` only, distinct, at least [`NEED`].
pub fn validate_relays(urls: &[String]) -> Result<(), String> {
    if urls.len() < NEED {
        return Err(format!("refused: {} drand relay(s); {NEED} must agree", urls.len()));
    }
    for (i, u) in urls.iter().enumerate() {
        if !u.starts_with("https://") || u.len() > 200 || u.contains(['?', '#', ' ']) {
            return Err(format!("refused: drand relay {u:?} is not a plain https:// URL"));
        }
        if urls[..i].contains(u) {
            return Err(format!("refused: drand relay {u} named twice"));
        }
    }
    Ok(())
}

/// A relay's host (for the log; never a path).
pub fn host_of(url: &str) -> String {
    url.split("://").nth(1).unwrap_or(url).split(['/', '?']).next().unwrap_or("").to_string()
}

/// The transform every drand outcall names.
#[ic_cdk::query]
fn drand_tf(args: ic_cdk::management_canister::TransformArgs) -> ic_cdk::management_canister::HttpRequestResult {
    let status = u32::try_from(args.response.status.0.clone()).unwrap_or(0);
    let body = normalise(status, &args.response.body).to_string().into_bytes();
    ic_cdk::management_canister::HttpRequestResult { status: candid::Nat::from(200u32), body, headers: vec![] }
}

/// One relay's answer for `round` on the IC (a replicated GET through [`drand_tf`]): the transformed JSON.
pub async fn fetch_ic(base: &str, round: u64) -> Result<Value, String> {
    use ic_cdk::management_canister::{http_request, HttpHeader, HttpMethod, HttpRequestArgs, TransformContext, TransformFunc};
    let req = HttpRequestArgs {
        url: url_of(base, round),
        method: HttpMethod::GET,
        body: None,
        max_response_bytes: Some(MAX_BYTES),
        transform: Some(TransformContext {
            function: TransformFunc(candid::Func { principal: ic_cdk::api::canister_self(), method: "drand_tf".into() }),
            context: vec![],
        }),
        headers: vec![HttpHeader { name: "accept".into(), value: "application/json".into() },
            HttpHeader { name: "user-agent".into(), value: "rook-canister/1".into() }],
        is_replicated: Some(true),
    };
    let res = http_request(&req).await.map_err(crate::canister::outcall_error)?;
    serde_json::from_slice(&res.body).map_err(|e| format!("transform output: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// evmnet round 1000 as api.drand.sh served it (`keeper/service/drand.ts` measured all four relays identical, 2026-09-26): the shape;
    /// the bytes here are a made-up 64-byte signature with its true sha256.
    fn beacon_json(round: u64, sig: &[u8]) -> Vec<u8> {
        json!({ "round": round, "randomness": hex::encode(Sha256::digest(sig)), "signature": hex::encode(sig) }).to_string().into_bytes()
    }

    /// 20 real evmnet beacons (`test/fixtures/drand-evmnet-20.json`, fetched 2026-09-21 for `DrandEvmnet`'s tests).
    fn real() -> Vec<(u64, Vec<u8>, String)> {
        let v: Value = serde_json::from_str(include_str!("../tests/fixtures/solana/drand-evmnet-20.json")).unwrap();
        v["beacons"].as_array().unwrap().iter().map(|b| (b["round"].as_u64().unwrap(), hex::decode(b["signature"].as_str().unwrap()).unwrap(),
            b["randomness"].as_str().unwrap().to_string())).collect()
    }

    #[test]
    fn every_real_evmnet_beacon_verifies_and_nothing_else_does() {
        let bs = real();
        assert_eq!(bs.len(), 20);
        for (round, sig, randomness) in &bs {
            assert_eq!(hex::encode(verify(*round, sig).unwrap_or_else(|e| panic!("round {round}: {e}"))), *randomness, "round {round}");
        }
        let (r, sig, _) = &bs[3];
        // another round's signature, a flipped bit, a coordinate over the modulus, the point at infinity
        assert!(verify(r + 1, sig).is_err());
        let mut bad = sig.clone();
        bad[63] ^= 1;
        assert!(verify(*r, &bad).is_err());
        assert!(verify(*r, &[0xff; 64]).is_err());
        assert!(verify(*r, &[0; 64]).is_err());
        assert!(verify(*r, &sig[..32]).is_err());
    }

    #[test]
    fn real_relays_agreeing_on_a_real_round_prove_it_and_a_forged_beacon_agreed_by_two_is_refused() {
        let (round, sig, randomness) = real().remove(5);
        let body = json!({ "round": round, "signature": hex::encode(&sig), "randomness": randomness }).to_string();
        let ok = |h: &str| (h.to_string(), Ok(normalise(200, body.as_bytes())));
        assert!(matches!(judge(round, &[ok("a"), ok("b")], NEED), Verdict::Proven { .. }));
        // two relays serving the same well-formed but forged beacon (randomness = sha256(signature)): the BLS check refuses it
        let forged = [9u8; 64];
        let fb = json!({ "round": round, "signature": hex::encode(forged), "randomness": hex::encode(Sha256::digest(forged)) }).to_string();
        let f = |h: &str| (h.to_string(), Ok(normalise(200, fb.as_bytes())));
        assert!(matches!(judge(round, &[f("a"), f("b")], NEED), Verdict::Disagree(ref w) if w.contains("does not verify")));
    }

    #[test]
    fn rounds_follow_rook_cores_clock() {
        assert_eq!(round_at(GENESIS - 1), 0);
        assert_eq!(round_at(GENESIS), 1);
        assert_eq!(round_at(GENESIS + 2), 1);
        assert_eq!(round_at(GENESIS + 3), 2);
    }

    #[test]
    fn two_relays_agreeing_prove_a_round_one_differing_refuses_it() {
        let (r5, sig, _) = real().into_iter().find(|b| b.0 == 3).unwrap();
        assert_eq!(r5, 3);
        let sig: [u8; 64] = sig.try_into().unwrap();
        let ok = |h: &str| (h.to_string(), Ok(normalise(200, &beacon_json(3, &sig))));
        let v = judge(3, &[ok("a"), ok("b"), ("c".into(), Ok(normalise(404, b"")))], NEED);
        assert!(matches!(v, Verdict::Proven { ref relays, .. } if relays.len() == 2), "{v:?}");
        // one relay alone is not yet
        assert_eq!(judge(3, &[ok("a"), ("b".into(), Err("down".into()))], NEED), Verdict::NotYet { have: 1 });
        // another signature: refused
        let other = ("c".to_string(), Ok(normalise(200, &beacon_json(3, &[8u8; 64]))));
        assert!(matches!(judge(3, &[ok("a"), ok("b"), other], NEED), Verdict::Disagree(_)));
        // randomness that is not sha256(signature): refused
        let lie = json!({ "round": 3, "randomness": hex::encode([1u8; 32]), "signature": hex::encode(sig) }).to_string();
        assert!(matches!(judge(3, &[ok("a"), ok("b"), ("c".into(), Ok(normalise(200, lie.as_bytes())))], NEED), Verdict::Disagree(_)));
        // another round served: refused
        assert!(matches!(judge(3, &[ok("a"), ok("b"), ("c".into(), Ok(normalise(200, &beacon_json(4, &sig))))], NEED), Verdict::Disagree(_)));
        // an HTTP 500 is an outage, not a lie
        assert!(matches!(judge(3, &[ok("a"), ok("b"), ("c".into(), Ok(normalise(500, b"")))], NEED), Verdict::Proven { .. }));
    }

    #[test]
    fn relays_are_plain_https_and_distinct() {
        let r = |v: &[&str]| validate_relays(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(r(&DEFAULT_RELAYS).is_ok());
        assert!(r(&["https://api.drand.sh"]).is_err());
        assert!(r(&["https://a", "http://b"]).is_err());
        assert!(r(&["https://a", "https://a"]).is_err());
        assert_eq!(host_of("https://api.drand.sh/x"), "api.drand.sh");
    }
}
