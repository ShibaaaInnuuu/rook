# Gas

Measured by `test/security/Gas.t.sol` (Phase 3), solc 0.8.28, optimizer 200 runs, via-IR, against mocks for Pons and the beacon.
Numbers are gas used by the call itself, cold storage included. 4663's block gas limit is 32,000,000.

| Call | Gas |
|---|---|
| harvest, first for a coin (queues it) | 254,636 |
| harvest, a coin already queued | 111,277 |
| postCap | 73,904 |
| declarePurchase | 232,877 |
| release, one payer | 242,466 |
| settleInscription (closes the purchase) | 300,278 |
| choose | 90,126 |
| openDraw | 110,152 |
| requestDraw | 33,078 |
| resolve (mock beacon; add ~143k for the real verifier) | 103,779 |
| claimWin, depth 16 | 70,592 |
| setDestination (bech32m checked on chain) | 146,679 |
| release, payers=1 | 253,293 |
| returnUnspent, payers=1 | 132,236 |
| settleRune, payers=1 | 238,628 |
| writeOff (bond slashed to every payer), payers=1 | 159,168 |
| release, payers=10 | 926,450 |
| returnUnspent, payers=10 | 452,015 |
| settleRune, payers=10 | 1,345,797 |
| writeOff (bond slashed to every payer), payers=10 | 823,836 |
| release, payers=32 | 2,571,959 |
| returnUnspent, payers=32 | 1,233,697 |
| settleRune, payers=32 | 4,052,258 |
| writeOff (bond slashed to every payer), payers=32 | 2,448,580 |
| release, payers=64 | 4,965,456 |
| returnUnspent, payers=64 | 2,370,689 |
| settleRune, payers=64 | 7,989,049 |
| writeOff (bond slashed to every payer), payers=64 | 4,811,844 |
| claimShare, depth=10 (2^depth holders) | 108,143 |
| claimShare, depth=16 (2^depth holders) | 112,692 |
| claimShare, depth=20 (2^depth holders) | 115,747 |

## Every loop, and its ceiling

| Loop | Bounded by | Worst case measured |
|---|---|---|
| `release` — one charge per paying coin | `MAX_PAYERS` = 64, else `TooManyPayers` | 4.97M |
| `settleRune` — one acquisition per paying coin | 64 | 7.99M (a quarter of a block) — this is what set the ceiling at 64; at 128 it was 15.9M |
| `returnUnspent`, `writeOff` — one credit per paying coin | 64 | 2.37M, 4.81M |
| `settleInscription` — picks the payer with the largest unmatched charge | 64 | under 0.5M |
| `isBlocked`, `_close`, `withdrawBond` — open purchases | `MAX_OPEN` = 3 | negligible |
| `payersFor` — a view over the queue | the number of queued coins | not called on chain |
| Merkle proofs | depth (20 = a million holders) | 116k |

No loop runs over deposits, holders, listings, coins or acquisitions. Before Phase 3 `release` ran over deposits, which anyone could
create for a wei (docs/phase3.md H1).

The real drand verifier adds about 143,000 gas to the first `resolve` of a round (worst of 20 real beacons: 142,994).
