
// ===== review round 7, part F: F6-L-1's margin once the pin's lag under the head is modelled =====
// The fake's pin is the head (`HOME_PIN + elapsed * 10`), so its `state_depth` is measured from the pin. On 4663 the provider's reach
// (6,216-6,234 blocks, probed 2026-09-28, poc/F/depth_scan.output.txt) is measured from ITS head, and HomeTx.pin is the fast pin,
// 1,024-2,047 blocks (+ the providers' spread) under the head at signing. So the reach seen from HomeTx.pin is 6,216 - L:
// 4,169 (L = 2,047) to 5,192 (L = 1,024). The first resolving tick comes 240 s + the rest of the cycle after the signing tick.
#[test]
fn r7_f_reach_from_the_pin_is_4169_to_5192_blocks() {
    for (depth, dt, want) in [(4_169u64, 300u64, "MINED"), (4_169, 420, "UNIDENTIFIED"), (5_192, 420, "MINED"), (5_192, 540, "UNIDENTIFIED")] {
        let (mut f, mut st) = hidden_deposit_live(Some(depth));
        let l = run_for(&mut f, &mut st, 1, dt).join("\n");
        let l7 = if want == "UNIDENTIFIED" { run_for(&mut f, &mut st, 7, 600).join("\n") } else { String::new() };
        let got = if l.contains("MINED") && l.contains("nonce count") { "MINED" } else if l7.contains("UNIDENTIFIED") { "UNIDENTIFIED" } else { "?" };
        println!("reach from the pin {depth}, first resolving tick {dt} s after signing: {got}");
        assert_eq!(got, want, "{l}\n{l7}");
    }
}
