# R3 check, contracts part (reviewer's own work)

## Round-2 contract items, re-attacked

| ID | Verdict | How it was checked |
| --- | --- | --- |
| contracts N-1 (Medium): (a) 1-wei target fresh freeze, (b) a pause freezes burns for a week | **Fixed** | The original R2 PoCs `R2Floor.t.sol` run unmodified against the new code (copies in `scratchpad/rook-audit/R3/orig-r2-pocs/`) now FAIL on the attack assertions: `no burn price: 132537322045440000 != 0` (a burn price exists under 8 days of 1-wei posts) and `an hour later: refused` (the burn goes through an hour into a pause). Variant `audit3 test_audit3_confidenceZeroPostsFreezeBurnsUntilAPause`: a keeper that posts confidence 0 (or nothing) still freezes burns for up to 7 d 1 h **until someone pauses it**; the pause now ends it at once. That is the documented "ordinary outage" residual, not a regression. |
| N-2 (Low) snapshot re-roll | **Fixed** (for the vectors R2 named) | Original `R2Draws.t.sol` unmodified: `bounded: 200 >= 200` (200 discarded announcements never land the chosen hour) and `a new snapshot time` fails (a guardian cancel after reveal keeps the time). `test_audit3_N2_reroll_by_void_and_cancel_is_dead`: 30 voids + a guardian cancel of an Open draw, one snapshot time throughout. Residual in Info A3-I2. |
| N-3 (Low) close during pause | **Fixed** for the close; residual A3-L1 for the hold | Original `R2Curve.t.sol`: `the close gate still says yes while paused` fails. The hold for an already-Queued grad ends at the unpause while the floor is still stale (A3-L1). |
| canister N-2, EVM side (LostRace) | **Fixed on chain** (gate) | `R3LateDelivery.t.sol` read and re-run; the gate is `refund` → `TooEarly` (grace) then `NotUndeliverable` (no attestation and < 30 d). `recordDelivery` stays open for a Resolved ticket whether or not it is marked, so a marked-then-delivered ticket is Delivered, never refunded. The residual is a false or premature attestation by the canister (canister part). |
