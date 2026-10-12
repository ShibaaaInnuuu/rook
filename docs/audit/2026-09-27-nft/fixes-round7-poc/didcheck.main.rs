//! Review round 6, part E: the HEAD .did against the canister's own encoder (every field of the round-5 fixture, both directions:
//! a field the Rust type has and the .did lacks, or the reverse, is reported), and service compatibility with 8ef82d6's .did.
use candid::types::value::IDLValue;
use candid::types::{Type, TypeInner};
use candid::IDLArgs;
use candid_parser::utils::{service_compatible, CandidSource};

fn fields(a: &IDLValue, b: &IDLValue, path: &str, out: &mut Vec<String>) {
    match (a, b) {
        (IDLValue::Opt(x), IDLValue::Opt(y)) => fields(x, y, path, out),
        (IDLValue::Opt(_), _) => out.push(format!("{path}: present on the wire, absent through the .did")),
        (IDLValue::Record(fs), IDLValue::Record(gs)) => {
            for f in fs {
                match gs.iter().find(|g| g.id.get_id() == f.id.get_id()) {
                    Some(g) => fields(&f.val, &g.val, &format!("{path}.{}", f.id), out),
                    None => out.push(format!("{path}.{}: in the Rust type, not in the .did", f.id)),
                }
            }
            for g in gs {
                if !fs.iter().any(|f| f.id.get_id() == g.id.get_id()) {
                    out.push(format!("{path}.{}: in the .did, not in the Rust type", g.id));
                }
            }
        }
        (IDLValue::Variant(v), IDLValue::Variant(w)) => fields(&v.0.val, &w.0.val, &format!("{path}::{}", v.0.id), out),
        (IDLValue::Vec(xs), IDLValue::Vec(ys)) => {
            for (i, (x, y)) in xs.iter().zip(ys.iter()).enumerate() {
                fields(x, y, &format!("{path}[{i}]"), out)
            }
        }
        _ => {}
    }
}

fn main() {
    let root = "/Users/jlohayon/Documents/Rook-nft/autobuyer/canister-rs";
    let did = std::fs::read_to_string(format!("{root}/etch_autobuyer.did")).unwrap();
    let old = std::fs::read_to_string(std::env::var("OLDDID").unwrap()).unwrap();
    println!("service_compatible(HEAD .did, OLD .did): {:?}", service_compatible(CandidSource::Text(&did), CandidSource::Text(&old)).map_err(|e| e.to_string()));
    let (env, _) = CandidSource::Text(if std::env::var("OLD").is_ok() { &old } else { &did }).load().expect("the .did parses");
    for (ty, what, path) in [("EthState", "eth-state-round7.hex", format!("{root}/tests/fixtures/eth-state-round7.hex")), ("EthState", "eth-state-round6.hex", format!("{root}/tests/fixtures/eth-state-round6.hex")),
        ("EthState", "eth-state-round5.hex", format!("{root}/tests/fixtures/eth-state-round5.hex")),
        ("EthState", "eth-state-round4.hex", format!("{root}/tests/fixtures/eth-state-round4.hex")),
        ("EthState", "eth-state-round2.hex", format!("{root}/tests/fixtures/eth-state-round2.hex"))] {
        let b = hex::decode(std::fs::read_to_string(&path).unwrap().trim()).unwrap();
        let wire = IDLArgs::from_bytes(&b).unwrap().args[0].clone();
        let t = Type::from(TypeInner::Var(ty.into()));
        match IDLArgs::from_bytes_with_types(&b, &env, &[t]) {
            Ok(a) => {
                let mut out = vec![];
                fields(&wire, &a.args[0], "", &mut out);
                println!("{}: {what}: {} bytes; differences {out:?}", if out.is_empty() { "ok" } else { "MISMATCH" }, b.len());
            }
            Err(e) => println!("MISMATCH: {what}: {e}"),
        }
    }
}
