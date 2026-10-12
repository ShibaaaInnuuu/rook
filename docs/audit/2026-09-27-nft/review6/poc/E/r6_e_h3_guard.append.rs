
// =====================================================================================================================================
// Review round 6, part E (E6-I-1): mutations the E5-I-2 semantic H3 guard does NOT catch. Appended to a private copy of
// tests/audit_nft_r5.rs (it uses that file's private `h3_check` / `src_files`). Each assertion PASSES while the weakness is present:
// the guard accepts a source whose H3 keying is broken. Nothing here is compiled from the mutated text; the guard reads text only.
// =====================================================================================================================================

#[test]
fn r6_e6_i1_the_h3_guard_misses_these_mutations() {
    let files = src_files();
    h3_check(&files).unwrap();
    let with = |f: &str, from: &str, to: &str| -> Vec<(String, String)> {
        let mut v = files.clone();
        let x = v.iter_mut().find(|(n, _)| n == f).unwrap();
        assert!(x.1.contains(from), "{from}");
        x.1 = x.1.replacen(from, to, 1);
        v
    };
    // (1) the deposit mint's `col` made per deposit: mint_key's text is unchanged, but the key AND the per-collection scope are now one
    //     per deposit (E4-H-1's shape, and the per-collection mint limit never binds). The guard does not resolve `col`.
    let m1 = with("ethflow.rs", "            let col = key_of(&c, chain);\n            let key = mint_key(st, &col, &d.collection);",
        "            let col = format!(\"{}:{}\", key_of(&c, chain), h(&d.tx));\n            let key = mint_key(st, &col, &d.collection);");
    assert!(h3_check(&m1).is_ok(), "(1) caught");
    // (2) the delivery key made per tick by a later mutation of a `let mut` binding (not a new `let`): the guard reads the `let` only
    let m2 = with("ethflow.rs", "            let key = format!(\"deliver:{}:{}\", h(&d.wrapper), d.ticket);\n",
        "            let mut key = format!(\"deliver:{}:{}\", h(&d.wrapper), d.ticket);\n            key.push_str(&format!(\":{}\", io.now()));\n");
    assert!(h3_check(&m2).is_ok(), "(2) caught");
    // (3) the RECORD side: every delivery recorded under one constant key (idempotent per key), so only the window's first delivery
    //     counts toward what went out and the per-day item limits never bind. The guard reads `outflow_check(` sites only.
    let m3 = with("ethflow.rs", "            io.outflow_record(&key, &col, Flow::Item, 1);\n            let nonce = st.line",
        "            io.outflow_record(\"deliver\", &col, Flow::Item, 1);\n            let nonce = st.line");
    assert!(h3_check(&m3).is_ok(), "(3) caught");
    // (4) a new ask through a path expression, a per-call key: `outflow_check)(` is not `outflow_check(`
    let mut m4 = files.clone();
    m4.iter_mut().find(|(n, _)| n == "ethflow.rs").unwrap().1.push_str(
        "\nasync fn ask_again<I: EthIo>(io: &mut I) -> Result<(), String> { let k = format!(\"x:{}\", io.now()); (I::outflow_check)(io, &k, \"c\", Flow::Mint, 1) }\n");
    assert!(h3_check(&m4).is_ok(), "(4) caught");
    // (5) a new site in a file under a sub-directory of src/: src_files() reads src/*.rs only
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/r6extra");
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(format!("{dir}/mod.rs"), "pub fn ask(n: u64) -> Result<(), String> { crate::canister::outflow_check(&format!(\"loop:{n}\"), \"c\", crate::outflow::Flow::Item, 1) }\n").unwrap();
    let got = h3_check(&src_files());
    std::fs::remove_dir_all(dir).unwrap();
    assert!(got.is_ok(), "(5) caught");
}
