// Review round 5, part A: option (c)'s queue plan against the timelock's rules. APPEND to a PRIVATE copy of
// autobuyer/timelock-rs/src/rules.rs, then: cargo test --lib r5a -- --nocapture. Uses only `State`'s public rules (the same calls
// `canister.rs` makes: propose, withdraw, take_ready, finish). Passes on 1dc2a61.
#[cfg(test)]
mod r5a {
    use super::*;
    const NS: u64 = 1_000_000_000;
    const H: u64 = 3600 * NS;
    fn p(n: u8) -> Principal { Principal::from_slice(&[n; 10]) }
    fn roles() -> Roles { Roles { proposers: vec![p(20), p(21)], guardians: vec![p(30), p(31), p(32)], veto_threshold: 2 } }
    fn cfg() -> Config { Config { target: p(1), roles: roles(), delay_secs: MIN_DELAY_SECS } }
    fn up(tag: u8) -> Action {
        let mut w = b"\0asm\x01\0\0\0".to_vec();
        w.extend_from_slice(&[7; 100]);
        Action::Upgrade { wasm: w, arg: vec![0x44, 0x49, 0x44, 0x4c, tag] }
    }

    #[test]
    fn option_c_queue_rules() {
        // install happened outside the timelock (the pre-handover reinstall): epoch 0. The outflow raise is queued at once.
        let mut s = State::new(cfg()).unwrap();
        let raise = s.propose(p(20), up(31), 0).unwrap().id;
        println!("outflow raise queued at t0: runnable at {} h", MIN_DELAY_SECS / 3600);
        // the same proposer cannot queue the caps raise beside it
        let e = s.propose(p(20), up(61), H).unwrap_err();
        println!("same proposer queues the caps raise while the outflow raise is pending: {e}");
        assert!(e.contains("already has proposal"));
        // another proposer can; whichever executes first makes the other stale (target epoch)
        let caps = s.propose(p(21), up(61), H).unwrap().id;
        assert!(s.take_ready(caps, MIN_DELAY_SECS * NS).unwrap_err().contains("may run in"), "the caps raise waits its own 72 h");
        assert!(matches!(s.take_ready(raise, MIN_DELAY_SECS * NS), Ok(Ready::Target(_))));
        s.finish(raise);
        let e = s.take_ready(caps, MIN_DELAY_SECS * NS + H).unwrap_err();
        println!("caps raise queued 1 h after the outflow raise, executed after it: {e}");
        assert!(e.starts_with("stale"));
        // the other order: the caps proposal ripe first; running it (the canister REFUSES caps-first: the install fails, the
        // timelock records Failed and keeps the epoch moved) makes the outflow raise stale too: nothing lands out of order, but the
        // raise must be proposed again and wait 72 h again
        let mut s = State::new(cfg()).unwrap();
        let caps = s.propose(p(21), up(61), 0).unwrap().id;
        let raise = s.propose(p(20), up(31), H).unwrap().id;
        assert!(matches!(s.take_ready(caps, MIN_DELAY_SECS * NS), Ok(Ready::Target(_))));
        s.finish(caps); // install_code trapped in the canister's post_upgrade (validate_custody): finish without untake
        let e = s.take_ready(raise, MIN_DELAY_SECS * NS + H).unwrap_err();
        println!("caps-first ran first and was refused by the canister; the outflow raise then: {e}");
        assert!(e.starts_with("stale"));
        // withdraw and re-queue: the delay starts again
        let mut s = State::new(cfg()).unwrap();
        let a = s.propose(p(20), up(31), 0).unwrap().id;
        s.withdraw(p(20), a, 10 * H).unwrap();
        let b = s.propose(p(20), up(31), 10 * H).unwrap();
        println!("withdrawn at 10 h and re-queued: runnable at {} h", (b.executable_ns / H));
        assert_eq!(b.executable_ns, 10 * H + MIN_DELAY_SECS * NS);
        // a proposal made BEFORE the launch install, if that install is itself a timelock upgrade, is stale at once
        let mut s = State::new(cfg()).unwrap();
        let install = s.propose(p(20), up(48), 0).unwrap().id;
        let early = s.propose(p(21), up(31), 0).unwrap().id;
        assert!(matches!(s.take_ready(install, MIN_DELAY_SECS * NS), Ok(Ready::Target(_))));
        s.finish(install);
        let e = s.take_ready(early, MIN_DELAY_SECS * NS).unwrap_err();
        println!("outflow raise queued beside a timelocked launch upgrade: {e}");
        assert!(e.starts_with("stale"));
        // no path shortens the delay: the guardians' pause / start / resume install nothing
        assert!(s.admits(p(30), "pause_target", None, 0) && !s.admits(p(30), "execute", Some(99), 0));
    }
}
