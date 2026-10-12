
// ===== review round 7, part E: mutations the round-6 H3 guard (E6-I-1) still accepts =====
#[test]
fn r7_e_h3_guard_new_mutations() {
    let files = src_files();
    h3_check(&files).unwrap();
    let with = |f: &str, from: &str, to: &str| -> Vec<(String, String)> {
        let mut v = files.clone();
        let x = v.iter_mut().find(|(n, _)| n == f).unwrap();
        assert!(x.1.contains(from), "{from}");
        x.1 = x.1.replacen(from, to, 1);
        v
    };
    let site = "            let key = format!(\"deliver:{}:{}\", h(&d.wrapper), d.ticket);\n";
    let mut res = Vec::new();
    // caught ones (controls): a helper fn, a macro, a struct field, a closure
    let helper = with("ethflow.rs", site, "            let key = deliver_key(&d, io.now());\n");
    res.push(("helper fn key", h3_check(&helper).is_err()));
    let mac = with("ethflow.rs", site, "            let key = deliver_key!(d);\n");
    res.push(("macro key", h3_check(&mac).is_err()));
    let mut field = files.clone();
    field.iter_mut().find(|(n, _)| n == "ethflow.rs").unwrap().1.push_str(
        "\nstruct K { key: String }\nasync fn ask_field<I: EthIo>(io: &mut I, k: K) -> Result<(), String> { io.outflow_check(&k.key, \"c\", Flow::Mint, 1) }\n");
    res.push(("struct-field key", h3_check(&field).is_err()));
    let mut clo = files.clone();
    clo.iter_mut().find(|(n, _)| n == "ethflow.rs").unwrap().1.push_str(
        "\nasync fn ask_clo<I: EthIo>(io: &mut I) -> Result<(), String> { let f = |io: &mut I, s: &str| io.outflow_check(s, \"c\", Flow::Mint, 1); f(io, \"x\") }\n");
    res.push(("closure", h3_check(&clo).is_err()));
    // (A) an inner-block decoy: the real key is per-time, the textually nearest `let key =` is the reviewed text in a closed block
    let decoy = with("ethflow.rs", site, "            let key = format!(\"deliver:{}:{}:{}\", h(&d.wrapper), d.ticket, io.now());\n            { let key = format!(\"deliver:{}:{}\", h(&d.wrapper), d.ticket);\n let _ = &key; }\n");
    let a = h3_check(&decoy);
    res.push(("(A) inner-block decoy shadow", a.is_err()));
    // (B) a type-annotated let (`let key: String =` is not a binding the guard reads) after a decoy `let key =`
    let typed = with("ethflow.rs", site, "            let key = format!(\"deliver:{}:{}\", h(&d.wrapper), d.ticket);\n            let _unused = &key;\n            let key: String = format!(\"deliver:{}:{}:{}\", h(&d.wrapper), d.ticket, io.now());\n");
    res.push(("(B) typed let after a decoy", h3_check(&typed).is_err()));
    // (C) key_of's body (the scope of mint_key's col and of the advance key) made per-call: not pinned
    let keyof = with("ethflow.rs", "    custodycap::evm_key(chain, &c.collection)\n}",
        "    format!(\"{}:{}\", custodycap::evm_key(chain, &c.collection), ic_cdk::api::time())\n}");
    res.push(("(C) key_of made per-call", h3_check(&keyof).is_err()));
    // (D) a module pulled from outside src/ (#[path]) : the guard reads src/ only
    let mut path = files.clone();
    path.iter_mut().find(|(n, _)| n == "lib.rs").unwrap().1.push_str("\n#[path = \"../extra/ask.rs\"] pub mod extra_ask;\n");
    res.push(("(D) #[path] module outside src/", h3_check(&path).is_err()));
    // (E) include! of a file outside src/
    let mut inc = files.clone();
    inc.iter_mut().find(|(n, _)| n == "ethflow.rs").unwrap().1.push_str("\ninclude!(\"../extra/ask2.rs\");\n");
    res.push(("(E) include! outside src/", h3_check(&inc).is_err()));
    for (n, caught) in &res {
        println!("{n}: {}", if *caught { "caught" } else { "ACCEPTED by the guard" });
    }
}
