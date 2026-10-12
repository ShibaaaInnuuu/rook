// Review round 5, part A: the exit rule (A4-L-1) as the canister holds it, and option (c), the operator's choice of 2026-09-28.
// APPEND to a PRIVATE copy of autobuyer/canister-rs/src/canister.rs (it is a child module of `canister`, so it reaches the private
// `validate_custody`, `check_cap_change` and `caps_of`), then:
//   cargo test --lib r5a -- --nocapture --test-threads=1
// Every test PASSES on 1dc2a61. The ones named `weakness_*` pass because the weakness is present.
#[cfg(test)]
mod r5a {
    use super::*;
    use crate::custodycap::{CapPolicy, CollectionCap};
    use crate::outflow::{Flow, Limits, Outflow, Verdict};

    const E: u128 = 1_000_000_000_000_000_000;
    // Milady, Azuki, MAYC, Pudgy, BAYC (nft-pilot.json order)
    const COLS: [&str; 5] = ["5af0d9827e0c53e4799bb226655a1de152a425a5", "ed5af388653567af2f388e6224dc7c4b3241c544",
        "60e4d786628fea6478f785a6d7e704777c86a7c6", "bd3531da5cf5857e7cfaa92426877b022e612cf8", "bc4ca0eda7647a8ab7c2061c2e118a18a936f13d"];
    const NAMES: [&str; 5] = ["milady", "azuki", "mayc", "pudgy", "bayc"];
    const J7: [u32; 5] = [16, 22, 17, 4, 2];
    const OPT_A: [u32; 5] = [13, 17, 13, 3, 2];

    fn key(i: usize) -> String {
        format!("1:0x{}", COLS[i])
    }
    fn caps(items: &[u32], global: Option<u32>) -> CapPolicy {
        let collections: Vec<CollectionCap> = items.iter().enumerate().map(|(i, &n)| CollectionCap {
            collection: key(i), max_items: n, max_value_wei: 15 * E, max_item_wei: 15 * E }).collect();
        let sum: u32 = items.iter().sum();
        CapPolicy { collections, global_max_items: global.unwrap_or(sum).max(1), global_max_value_wei: 15 * E * items.len() as u128 }
    }
    fn lim(per: u32, global: u32) -> Limits {
        Limits { per_collection_items: per, global_items: global, per_collection_mints: 3, global_mints: 6, global_value_wei: 25 * E, trip_factor: 3 }
    }
    fn init(c: CapPolicy, l: Limits) -> Init {
        Init {
            chain_id: 4663, vault: "0x0000000000000000000000000000000000000001".into(), rpc_urls: vec!["https://a".into(), "https://b".into()],
            btc_api_urls: vec!["https://c".into(), "https://d".into()], key_name: "dfx_test_key".into(), accept_production_key_name: false,
            bridge_allowlist: vec![], btc_network: "signet".into(), operator: None, wrapper: None, evm_chains: None,
            custody_caps: Some(c), outflow_limits: Some(l), holder_deposits: None, wrapper_from_block: None, frog_funding: None, eth_nft: None,
        }
    }
    fn show(r: &Result<(), String>) -> String {
        match r { Ok(()) => "ACCEPTED".into(), Err(e) => format!("REFUSED ({})", e.split(':').next().unwrap_or("")) }
    }

    /// 1. The arithmetic, as a grid, printed for the Python side (`exit_agreement.py` runs golive-check's EXIT_PY on the same lines).
    /// Format: `GRID <items comma> <global cap or -> <per> <global> <verdict>`; verdict is `ok` / `exit` (the exit rule refused) /
    /// `other:<why>` (refused before the exit rule: a malformed policy or limits).
    #[test]
    fn grid_for_the_python_side() {
        let mut lines = vec![];
        let item_sets: Vec<Vec<u32>> = vec![
            J7.to_vec(), OPT_A.to_vec(), vec![24, 24], vec![24, 25], vec![12, 12, 12, 12], vec![13, 12, 12, 12], vec![0, 48], vec![0, 0],
            vec![1], vec![48], vec![49], vec![25, 24], vec![16, 16, 16], vec![17, 16, 16], vec![23, 24, 2], vec![0, 24, 24]];
        for items in &item_sets {
            for gcap in [None, Some(48u32), Some(47), Some(24), Some(0)] {
                for (per, glob) in [(12u32, 24u32), (12, 31), (12, 30), (3, 6), (12, 12), (24, 24), (0, 24), (12, 0), (13, 24)] {
                    let mut c = caps(items, None);
                    if let Some(g) = gcap { c.global_max_items = g; }
                    let r = validate_custody(&init(c, lim(per, glob)));
                    let v = match &r {
                        Ok(()) => "ok".to_string(),
                        Err(e) if e.contains("in all and lets") => "exit".to_string(),
                        Err(e) if e.contains("holders could not leave") => "exit".to_string(),
                        Err(e) => format!("other:{}", e.replace(' ', "_").chars().take(40).collect::<String>()),
                    };
                    let is: Vec<String> = items.iter().map(|x| x.to_string()).collect();
                    lines.push(format!("GRID {} {} {per} {glob} {v}", is.join(","), gcap.map_or("-".into(), |g| g.to_string())));
                }
            }
        }
        for l in &lines { println!("{l}"); }
    }

    /// 2. Option (c) as the canister's upgrade check sees it (check_cap_change = validate_custody on the KEPT argument, then the ×4
    /// rule for caps and limits against the SAVED one).
    #[test]
    fn option_c_sequence_through_the_upgrade_check() {
        let a = init(caps(&OPT_A, None), lim(12, 24));
        let j7_at_24 = init(caps(&J7, None), lim(12, 24));
        println!("install J7 61 @ 24/day:                    {}", show(&check_cap_change(None, &j7_at_24)));
        assert!(check_cap_change(None, &j7_at_24).is_err());
        println!("install (a) 48 @ 24/day:                   {}", show(&check_cap_change(None, &a)));
        assert!(check_cap_change(None, &a).is_ok());
        // an install may not start global_items above 24 (x4 stage 0's 6)
        let a31 = init(caps(&OPT_A, None), lim(12, 31));
        println!("install (a) 48 @ 31/day:                   {}", show(&check_cap_change(None, &a31)));
        assert!(check_cap_change(None, &a31).unwrap_err().contains("global_items"));
        // step 1: outflow first
        let s1 = init(caps(&OPT_A, None), lim(12, 31));
        println!("(a) -> 48 @ 31 (outflow first):            {}", show(&check_cap_change(Some(&a), &s1)));
        assert!(check_cap_change(Some(&a), &s1).is_ok());
        // step 2: caps back to J7
        let s2 = init(caps(&J7, None), lim(12, 31));
        println!("48 @ 31 -> 61 @ 31 (caps second):          {}", show(&check_cap_change(Some(&s1), &s2)));
        assert!(check_cap_change(Some(&s1), &s2).is_ok());
        // caps first: refused (checked against the NEW outflow, which is still 24)
        println!("(a) -> 61 @ 24 (caps first):               {}", show(&check_cap_change(Some(&a), &j7_at_24)));
        assert!(check_cap_change(Some(&a), &j7_at_24).is_err());
        // one upgrade doing both: ACCEPTED (the check reads the new pair only)
        println!("(a) -> 61 @ 31 (both in one upgrade):      {}", show(&check_cap_change(Some(&a), &s2)));
        assert!(check_cap_change(Some(&a), &s2).is_ok());
        // the x4 rule: 24 -> 96 is one step, 97 is not; 31 is reachable in one step
        println!("(a) -> 48 @ 96:                            {}", show(&check_cap_change(Some(&a), &init(caps(&OPT_A, None), lim(12, 96)))));
        assert!(check_cap_change(Some(&a), &init(caps(&OPT_A, None), lim(12, 96))).is_ok());
        println!("(a) -> 48 @ 97:                            {}", show(&check_cap_change(Some(&a), &init(caps(&OPT_A, None), lim(12, 97)))));
        assert!(check_cap_change(Some(&a), &init(caps(&OPT_A, None), lim(12, 97))).is_err());
        // per-collection may go 12 -> 48 in one step too (J7's Azuki 22 in one day needs 22)
        println!("(a) -> 48 @ per 22 / 31:                   {}", show(&check_cap_change(Some(&a), &init(caps(&OPT_A, None), lim(22, 31)))));
        // lowering the outflow later while the caps say 61: refused
        let low = init(caps(&J7, None), lim(12, 24));
        println!("61 @ 31 -> 61 @ 24 (outflow lowered):      {}", show(&check_cap_change(Some(&s2), &low)));
        assert!(check_cap_change(Some(&s2), &low).is_err());
        // option (b) through the canister: a binding global cap of 48 over J7's per-collection caps
        let b = init(caps(&J7, Some(48)), lim(12, 24));
        println!("install (b) J7 per-collection, global 48:  {}", show(&check_cap_change(None, &b)));
        assert!(check_cap_change(None, &b).is_ok());
    }

    /// 3. WEAKNESS (A5-L-1): the exit rule is checked on the caps NAMED, never on what custody HOLDS. With 61 items held under
    /// (c)'s end state, one upgrade that lowers the caps and the outflow together ("back to stage 0", a lowering the rules always
    /// allow) passes; after it lands custody still holds 61 and lets 6 a day out: 12 can leave within the next proposal's delay.
    #[test]
    fn weakness_lowering_caps_and_outflow_together_passes_while_custody_holds_more() {
        let end_c = init(caps(&J7, None), lim(12, 31));
        let stage0ish = init(caps(&[3, 3, 3, 2, 1], None), lim(3, 6));
        let r = check_cap_change(Some(&end_c), &stage0ish);
        println!("61 @ 31 -> caps 12 @ 6/day (custody still holds 61): {}", show(&r));
        assert!(r.is_ok(), "the check never reads the book");
        // what the limiter then lets out of 61 held items in the 48 h the rule counts on (every holder asks at t0)
        let l = lim(3, 6);
        let mut o = Outflow::default();
        let mut left: Vec<(usize, u32)> = J7.iter().enumerate().map(|(i, &n)| (i, n)).collect();
        let mut out = 0u32;
        let mut t = 0u64;
        while t < 48 * 3600 {
            for (i, n) in left.iter_mut() {
                while *n > 0 {
                    let k = format!("deliver:{}:{}", i, *n);
                    match o.check(&l, &k, &key(*i), Flow::Item, 1, t) {
                        Verdict::Allow => { o.record(&k, &key(*i), Flow::Item, 1, t); *n -= 1; out += 1; }
                        _ => break,
                    }
                }
            }
            t += 600;
        }
        println!("  held 61, limits 3/6 a day: {out} leave within 48 h, {} cannot", 61 - out);
        assert!(out <= 12);
    }

    /// Deliveries as `ethflow::eth_act` does them: every tick (10 min), the waiting ones in deadline order (= the order their tickets
    /// resolved), each asking the real H3 limiter; one that waits does not hold up the others. Returns (items left per collection
    /// after `hours`, delivered).
    fn run(order: &[usize], held: &[u32], l: &Limits, hours: u64) -> (Vec<u32>, u32) {
        let mut o = Outflow::default();
        let mut next = held.to_vec(); // per collection, how many tickets are still waiting
        let mut waiting: Vec<(usize, u32)> = vec![]; // (collection, ticket no.) in deadline order
        let mut seen = vec![0u32; held.len()];
        for &c in order { seen[c] += 1; waiting.push((c, seen[c])); }
        assert_eq!(seen, held.to_vec(), "the order names every held item once");
        let mut out = 0;
        let mut t = 0u64;
        while t < hours * 3600 {
            // eth_act signs one delivery a tick: the first waiting one (deadline order) the limiter allows
            let mut done = None;
            for (idx, (c, n)) in waiting.iter().enumerate() {
                let k = format!("deliver:{c}:{n}");
                match o.check(l, &k, &key(*c), Flow::Item, 1, t) {
                    Verdict::Allow => { o.record(&k, &key(*c), Flow::Item, 1, t); done = Some(idx); break; }
                    Verdict::Wait { .. } => continue,
                    Verdict::Frozen(cause) => panic!("tripped: {cause:?}"),
                }
            }
            if let Some(idx) = done { let (c, _) = waiting.remove(idx); next[c] -= 1; out += 1; }
            t += 600;
        }
        (next, out)
    }
    fn order(first: &[usize], held: &[u32]) -> Vec<usize> {
        // `first` collections' tickets first (interleaved), the rest after, each collection's in turn
        let mut v = vec![];
        let mut rem = held.to_vec();
        for pass in [first.to_vec(), (0..held.len()).filter(|c| !first.contains(c)).collect::<Vec<_>>()] {
            loop {
                let mut any = false;
                for &c in &pass { if rem[c] > 0 { v.push(c); rem[c] -= 1; any = true; } }
                if !any { break; }
            }
        }
        v
    }
    fn fmt(left: &[u32]) -> String {
        left.iter().enumerate().filter(|(_, &n)| n > 0).map(|(i, n)| format!("{} {n}", NAMES[i])).collect::<Vec<_>>().join(", ")
    }

    /// 4. WEAKNESS (A5-L-2): "each collection alone fits AND the whole custody fits" is necessary, not sufficient, under the canister's
    /// deadline-first delivery order: when Milady's and MAYC's tickets resolve first, they take day 1's global room and Azuki gets
    /// only its day-2 twelve. Every A4-L-1 option the canister and golive-check accept leaves items for the margin day.
    #[test]
    fn weakness_the_check_passes_but_the_delivery_order_leaves_items_past_the_usable_days() {
        let cases: Vec<(&str, Vec<u32>, Limits, u64)> = vec![
            ("(a) at install: 13/17/13/3/2 @ 12/24", OPT_A.to_vec(), lim(12, 24), 48),
            ("(c) interim: 13/17/13/3/2 @ 12/31", OPT_A.to_vec(), lim(12, 31), 48),
            ("(c) end: J7 16/22/17/4/2 @ 12/31", J7.to_vec(), lim(12, 31), 48),
            ("(b) held at the 48 cap: 16/22/10/0/0 @ 12/24", vec![16, 22, 10, 0, 0], lim(12, 24), 48),
            ("(d) J7 @ 12/24, 96 h delay (72 h usable)", J7.to_vec(), lim(12, 24), 72),
        ];
        for (name, held, l, hours) in cases {
            // the rule's own verdict for this pair (for (d) with its 3 usable days)
            let total: u32 = held.iter().sum();
            let days = (hours / 24) as u32;
            let fits = held.iter().all(|&n| n.div_ceil(l.per_collection_items) <= days) && total.div_ceil(l.global_items) <= days;
            let (fair_left, _) = run(&order(&[1], &held, ), &held, &l, hours); // Azuki first
            let (adv_left, adv_out) = run(&order(&[0, 2, 3, 4], &held), &held, &l, hours); // Azuki last
            println!("{name}: the rule says {}; Azuki's tickets first: left {:?}; Azuki's last: {adv_out} of {total} out in {hours} h, left: {}",
                if fits { "fits" } else { "does NOT fit" }, fair_left.iter().sum::<u32>(), fmt(&adv_left));
            assert!(fits, "{name}: the rule accepts it");
            assert_eq!(fair_left.iter().sum::<u32>(), 0);
            assert!(adv_left.iter().sum::<u32>() > 0, "{name}: an order exists that leaves items behind");
        }
        // what does fit J7 under every order at 48 h: per-collection limit at least the largest collection (the per-collection limit
        // never binds, any work-conserving order is optimal), or a global limit that covers Azuki's 12 beside every other's day-1 max
        for (per, glob) in [(22u32, 31u32), (12, 42)] {
            let l = lim(per, glob);
            let mut worst = 0;
            for first in [vec![0usize, 2, 3, 4], vec![0, 2], vec![2, 0], vec![0], vec![2], vec![3, 4, 0, 2]] {
                let (left, _) = run(&order(&first, &J7), &J7, &l, 48);
                worst = worst.max(left.iter().sum::<u32>());
            }
            println!("J7 @ {per}/{glob}: worst left over the orders tried: {worst}");
            assert_eq!(worst, 0);
        }
    }
}
