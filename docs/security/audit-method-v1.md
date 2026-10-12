# How this codebase gets audited, and where the method came from

Written 2026-09-22, after two internal review passes had already been done by reading the code and comparing it to a
competitor's. This document exists because that method has a known blind spot — it can only find what somebody thought to
look at — and the third pass was run against published professional practice instead of against intuition. What follows is
the rubric, its sources, and an honest account of what adopting it changed.

## Why the first two passes were not enough

`internal-review-v1.md` says it plainly: a reviewer who believed something while writing `release()` still believes it while
reviewing `release()`. Both earlier passes found real defects, including a critical one. But both worked outward from the
code, and both rated findings against *observed* behaviour. That is precisely the failure that produced this document's
headline result: the clock finding was **already recorded, as a Low**, because measured sequencer drift was 1.3 seconds.
Nobody had checked what the platform *permits*, which is an hour — exactly the length of the window it protects.

Reading the specification of the chain you deploy on is not intuition. It is homework, and it had not been done.

## The sources

| Source | What it contributes |
|---|---|
| [Trail of Bits, *Blockchain Maturity Evaluation*](https://github.com/crytic/building-secure-contracts/blob/master/development-guidelines/code_maturity.md) | The nine categories below. This is the spine of the review. |
| [Trail of Bits, *Development Guidelines*](https://secure-contracts.com/development-guidelines/guidelines.html) | Documentation and specification expectations; keep code off-chain; avoid assembly; migration over upgradeability; post-deployment duties (monitoring, a published security contact, an incident-response plan). |
| [OWASP Smart Contract Top 10 (2025)](https://owasp.org/www-project-smart-contract-top-10/) | A prevalence-ranked checklist — access control, oracle manipulation, logic errors, input validation, reentrancy, unchecked external calls, flash loans, over/underflow, insecure randomness, DoS. Drawn from 149 incidents and $1.42 bn of 2024 losses. |
| [Arbitrum: block numbers and time](https://docs.arbitrum.io/build-decentralized-apps/arbitrum-vs-ethereum/block-numbers-and-time) | **The chain's own specification.** Timestamps are the sequencer's, bounded to 24 h behind and **1 h ahead** of wall-clock; `block.number` is the parent chain's; timing is "reliable in the longer term… unreliable in the shorter term". |
| [Arbitrum: Solidity support](https://docs.arbitrum.io/build-decentralized-apps/arbitrum-vs-ethereum/solidity-support) | `block.difficulty`/`prevrandao` hardcoded to 1; `blockhash` is "cryptographically insecure"; `msg.sender` aliasing for parent-chain messages. |
| [Arbitrum: the sequencer](https://docs.arbitrum.io/how-arbitrum-works/sequencer) | It can censor and reorder; force-inclusion via the delayed inbox is the escape hatch, on a 24-hour window. |
| Slither 0.11.6 | Mechanical detectors, as a floor rather than a result. |

## The nine categories, and what each asked of this codebase

Trail of Bits' maturity model, applied. This is the checklist the third pass actually walked.

1. **Arithmetic** — overflow, rounding, precision. Which direction does each rounding favour, and is that the safe side? Is every truncating division documented? Are the edge cases tested, not just the happy path?
2. **Auditing** — does every critical operation emit an event? Is there off-chain monitoring? A documented incident response? Something that would *notice*?
3. **Authentication / access control** — least privilege; roles differentiated and documented; **revocation possible**; two-step for sensitive operations. Build the matrix: every external function, and exactly who may call it.
4. **Complexity management** — small functions, clear scope, no duplicated logic, input validation legible at the call site.
5. **Decentralization** — what can a privileged actor do unilaterally? Which parameters can change, how fast, and can a user opt out? What is immutable and is that the right side of the trade?
6. **Documentation** — is there a plain-English description, an architecture diagram, a state machine, an accurate NatSpec, and a written statement of what is *not* protected?
7. **Transaction-ordering risks** — MEV, front-running, and the order of two permissionless calls. Are ordering risks identified, mitigated, and *tested*?
8. **Low-level manipulation** — assembly, bitwise work, raw calls. Justified? Commented? Differentially fuzzed against a high-level reference?
9. **Testing and verification** — coverage, CI, integration tests, and whether the campaigns can detect what they claim to.

To which this system needs a tenth, because it is not on Ethereum:

10. **Chain-specific assumptions** — every value the EVM gives you that this chain defines differently from mainnet, and every assumption built on one. On Nitro that is `block.timestamp`, `block.number`, `blockhash`, `block.difficulty`/`prevrandao`, `msg.sender` aliasing, gas accounting, and the sequencer's powers.

## What the rubric changed, concretely

- **Category 10 produced the pass's most severe finding.** Every waiting period in the system — the public hour before money moves, the fifteen-minute draw challenge, the hour a winner has to correct a Bitcoin address — was measured on `block.timestamp` alone, on a chain whose sequencer may stamp a block an hour into the future. See `audit-v2.md` S1. It is fixed in `src/lib/Wait.sol`, against a rate measured on 4663 rather than assumed.
- **Category 3 produced the type-confusion finding.** Building the access-control matrix meant enumerating every external function, which surfaced two contracts casting a registry-supplied vault address to one concrete vault type and calling it. See `audit-v2.md` S4.
- **Category 9 produced the correction to the invariant campaigns.** "Whether the campaigns can detect what they claim to" is a question the earlier passes never asked of themselves; asking it is how C1's survival through 200,000 handler calls got explained, and why the v2 campaign now inspects revert data.
- **Category 2 is the weakest area and is not fixed.** There is no monitoring, no published security contact, and no incident-response plan. Trail of Bits lists all three as deployment prerequisites.

## What the method still does not give

Slither found nothing real — six detectors fired, every one a false positive on inspection (documented in `audit-v2.md`). That is the expected result for a codebase with no assembly, no delegatecall and no upgrade path, and it is worth saying because a clean static-analysis run is often mistaken for a clean audit. Every finding in both passes came from reading, from a specification, or from a competitor's source.

Nor does a rubric make this an audit. It is a structured self-review by the author of the code, and the category that the
structure cannot supply is the one Trail of Bits puts first in their own guidance: an independent reader. This document
makes the self-review legible to that reader; it does not replace them.
