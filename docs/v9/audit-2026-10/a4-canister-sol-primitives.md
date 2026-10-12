# A4: the canister's Solana primitives

| | |
|---|---|
| Area | A4: `solkey.rs`, `soltx.rs`, `solsend.rs`, `solrpc.rs`, `solread.rs`, `solix.rs`, the parts of `solhome.rs` that drive the line (`SolLine`, turn queue, `open_line`, `sign`, `tick_line`, `migrate`, `sol_clear_incident`), `Init.home` validation and restore in `canister.rs`, `etch_autobuyer.did` |
| Crate | `autobuyer/canister-rs` |
| Baseline | `f2c24e0` on `golive-2026-10-08` (2026-10-10) |
| Out of scope | The flows that use these primitives (`ethflow` home_sol, `frogsol`, `relaysol`, `convflow`): that is A5. Two colluding RPC providers. |
| PoC tests | `autobuyer/canister-rs/tests/audit_a4_line.rs` (new file; no existing source was modified) |
| Result | 2 High, 2 Medium, 2 Low, 10 Informational |

## 1. Threat model

**Assets**

1. **The threshold Ed25519 key's signing power.** The key on path `b"sol"` is the fee payer. It is also the custodian and poster on rook_core (`advance`, `settle_advance`, `item_arrived`, `return_advance`, `mint_item`, `record_delivery`, `note_sent`, `mark_undeliverable`, `post_floor`), the authority of the line's nonce account, and the depositor into Relay.
2. **The line's invariant.** It is "sign once, save the bytes, rebroadcast until they land, at most one lands". Every flow treats a step as idempotent because of it.
3. **The durable-nonce account.** Its address is `create_with_seed(me, nonce_seed, System)`.
4. **What the canister believes about Solana.** This covers account state at `finalized`, signature statuses, the nonce value, the finalized height, simulation verdicts, and each step's outcome as reported to its flow.
5. **Cycles.**

**Actors**

| Actor | Capability | In scope |
|---|---|---|
| One malicious or compromised RPC provider | Any answer to any request it receives. It keeps every signed transaction it is sent. | yes |
| Both providers colluding | Forge any agreed read | **no** (stated: agreement of ≥ 2 is the trust root, as on the EVM home) |
| An IC node operator | Sees every outcall (URLs with API keys, bodies, signed bytes). As the single replica that runs a **non-replicated** outcall (`sendTransaction`), it controls that outcall's answer. | yes |
| Anyone on Solana | Can transfer lamports to any address, submit any signed bytes they hold, front-run, and call rook_core's permissionless instructions (`mark_served`, `fund_reserve`, wake requests, redemptions) | yes |
| Anyone on the IC | Can call public queries (`sol_line`, `log`, `sol_tf`, ...) | yes |
| A malicious account owner | Can craft account data at addresses the canister reads, unless it does not own them | yes |
| The operator | Calls `sol_clear_incident` and `sol_address`, and passes upgrade arguments (through the timelock) | trusted, but checked for foot-guns |

## 2. Findings

| ID | Title | Severity | PoC |
|---|---|---|---|
| A4-01 | "Given up unforwarded" is believed from send answers that prove nothing: an abandoned step can land after the line reported it "never landed" | **High** | 3 failing tests |
| A4-02 | Anyone can squat the line's durable-nonce address with one transfer (~0.0009 SOL); the default line then never opens and the seed cannot be changed | **High** | failing test |
| A4-03 | Blockhash fallback: after 64 expiries every tick is refused, and nothing recovers the line | Medium | failing test |
| A4-04 | Blockhash fallback: "can never land" trusts a status read that may be older than the height read. Two landings are possible. | Medium | failing test |
| A4-05 | Decoders accept any account at least as long as the fields they read: a rook_core layout change is misread silently | Low | none |
| A4-06 | One provider (or an honest outage) can force the full-cap `getProgramAccounts` re-ask on every call | Low | none |
| A4-07 | The signing guard checks signers and size, not what is signed (no program or instruction allowlist) | Info | none |
| A4-08 | Keyed provider URLs: node operators see them, and IC reject texts may carry them into the public `log` | Info | none |
| A4-09 | compact-u16: the reader accepts non-canonical forms and the writer truncates silently | Info | none |
| A4-10 | The v0 / address-lookup-table compiler trusts table contents that nothing reads or checks today | Info | none |
| A4-11 | `decode_nonce` ignores the `Versions` tag | Info | none |
| A4-12 | The fallback's expiry margin is zero under the 151-hash age rule | Info | none |
| A4-13 | `restore_home` silently ignores identity fields in an upgrade argument; `validate` checks the argument's cluster, not the kept one | Info | none |
| A4-14 | `tick_line` writes `nonce_account = None` when `sol_me` fails at the moment the opening lands | Info | none |
| A4-15 | The outcome window and the "no outcome means not done" rule are safe only while each flow has at most one step out | Info | none |
| A4-16 | No compute-unit price is ever set | Info | none |

---

### A4-01: "Given up unforwarded" is believed from send answers that prove nothing (High)

**Files:**
- `src/solsend.rs:291-310` (`PREFLIGHT_REFUSALS`, `preflight_refused`)
- `src/solsend.rs:335-349` (first send)
- `src/solsend.rs:391-402` (`abandon_refused`)
- `src/solhome.rs:532-548` (`tick_line`, Preflight arm)
- `src/solrpc.rs:556-564` (`send`, `replicated: false`)
- `src/solhome.rs:288-291` (`sol_line` public query)

**Description.** On the first send, if every provider's answer has a preflight-refusal class, the line concludes the bytes were "forwarded nowhere". `tick_line` then calls `abandon_refused`, which drops the `Saved` record and its signature history. It also reports the step to its flow as `LineOutcome { ok: None }` ("would not succeed now ... given up unforwarded"), and the flow takes the step back and signs it again from a fresh read. The premise is not established:

1. **The answers are not agreed.** `sendTransaction` is deliberately a non-replicated outcall (Q3 §4.8). Its transformed answer is whatever the single replica that ran it returns, so one malicious node can forge "every provider refused" for bytes the providers actually forwarded.
2. **A provider keeps what it is sent.** A malicious provider answering "refused" may still hold the bytes and submit them later.
3. **The bytes are public anyway.** `sol_line` is an unguarded query that returns `Saved.tx`, the signed wire transaction. The bytes are readable from the moment the first send's await commits the record.
4. **Abandoning does not invalidate the bytes:**
   - Under the durable nonce, the abandoned bytes stay valid until the nonce moves.
   - Under the fallback, they stay valid for up to 150 blocks.
   - The abandoned signature leaves `history` (and `sol_seen`), so a later landing of it is not recognised as ours.

The comment at `solsend.rs:393` ("At most one of the two can still land: the new intent is signed on the same nonce") holds only if nobody lands the old bytes first.

**Impact.**
- **Durable nonce, and the abandoned bytes land successfully** (the node forged the refusal, or the state flips back):
  - The step happened, but its flow was told "never landed" and signs it again.
  - The nonce has moved, so the replacement is signed on the new nonce and also lands.
  - For a non-idempotent step (Relay `deposit_native`, `advance`, a transfer), that is a double execution, unless the flow's fresh read happens to see the first landing. That is A5's question; the primitive gave it the false input.
- **Durable nonce, and the abandoned bytes land *failed*:**
  - A failed nonced transaction still advances the nonce, so the replacement signed on that same nonce can never land.
  - The line raises an **INCIDENT**, which halts every Solana flow (Eth, Frog, Custody) until the operator clears it.
  - Anyone can do this: they only need the public bytes and a Solana submission. This also refutes the design claim that a moved nonce means "a bug or a compromised key".
- **Blockhash fallback:** the old bytes and the replacement, signed on a fresh blockhash, both land with no incident at all. The line's central invariant is broken silently.
- **Liveness:** a forged `insufficient_funds` answer pages SEV1 and keeps the bytes in "simulated again next tick" mode, never resent without preflight, for as long as the forging continues.

**Exploit sketches.**
- **(a) Malicious node:**
  1. It runs the first send and lets both providers forward.
  2. It returns `{"class":"error",...}` for both, and does so again at the next tick's re-send.
  3. The step is abandoned; the transaction lands; the flow re-signs; both land.
- **(b) Anyone, no node involvement:**
  1. A permissionless rook_core call (e.g. `mark_served` of the same request, or `fund_reserve`) finalizes between the canister's simulation and its first send's preflight, so the preflight genuinely fails.
  2. The attacker reads `Saved.tx` from `sol_line`.
  3. After the flow re-signs a different replacement on the same nonce, the attacker submits the old bytes with `skipPreflight`. They land failed, the nonce advances, and the result is an INCIDENT.

**PoC.** `tests/audit_a4_line.rs`:
- `poc_a4_01_abandoned_step_lands_and_its_replacement_lands_too_durable_nonce`: fails with "one intent, 2 successful landings (the line said the first never landed)".
- `poc_a4_01_abandoned_step_lands_and_its_replacement_lands_too_blockhash_fallback`: fails with "2 successful landings under the blockhash fallback".
- `poc_a4_01_anyone_turns_an_abandoned_step_into_an_incident`: fails with "a stranger's submission of abandoned public bytes halts the line: Some(\"the nonce account moved under intent 1 ...\")".

**Recommendation.** Never let a send's answer authorise "it did not happen" (Q3 §4.2 already says so for landings). Keep `preflight_refused` as an operator-visible `line.error` only. When a saved step must be given up:

- **Durable nonce: burn the nonce first.**
  1. Sign and save an `AdvanceNonceAccount`-only transaction on the same nonce, under the same intent, and add its signature to `history`.
  2. Report the step as abandoned (`ok: None`) only when the burn is the signature that finalizes.
  3. If the original finalizes instead, report it as landed, with its real `ok`.
- **Fallback:** keep the record until expiry is proven (`can_never_land`), then report `ok: None`.

In both cases keep every signature that was ever signed in `history` until its fate is proven.

---

### A4-02: Anyone can squat the line's durable-nonce address (High)

**Files:**
- `src/solhome.rs:369-399` (`open_line`, especially 372-377)
- `src/solsend.rs:198-209` (`nonce_account_of`, `open_nonce_ixs`)
- `src/soltx.rs:176-189` (`create_with_seed`)
- `src/canister.rs:176-186` (`restore_home` keeps `nonce_seed`)

**Description.** The nonce account's address is `sha256(me ‖ nonce_seed ‖ System)`. The line opens it with `SystemInstruction::CreateAccountWithSeed`, and the System Program refuses that instruction when the target already holds lamports (`AccountAlreadyInUse`). Anyone can make an address hold lamports with a plain transfer of at least the 0-byte rent-exempt minimum (890,880 lamports ≈ 0.0009 SOL), which leaves a System-owned account with empty data there. From then on:

1. The opening transaction fails, or its preflight is refused and it is abandoned.
2. On every later `ready()`, `open_line` finds an account that is not an initialized nonce of ours and returns "exists and is not an initialized nonce account of ours: an operator looks".
3. The seed is identity: `restore_home` keeps the saved `nonce_seed` whatever the upgrade argument says. No upgrade can move the line to a fresh address.

The address is easy to learn:
- `me` is public once the canister is funded.
- The seed is public as soon as the opening's `Saved` record appears in the `sol_line` query, which is readable during the send outcall's await, seconds before any provider forwards it. The seed is also public if it was ever published with the install argument.

**Impact.** The default (durable-nonce) Solana home never gets a line, so no flow can sign anything: no advance, settle, mint, `post_floor`, `record_delivery` or `return_advance`. Lamports already advanced and items in transit wait until the operator upgrades with `blockhash_fallback = true`. That moves every flow onto the fallback, with its 40-60 s validity and with A4-01, A4-03 and A4-04. Cost to the attacker: about $0.15, once, possibly before launch.

**PoC.** `poc_a4_02_a_transfer_to_the_nonce_address_blocks_the_durable_nonce_line_for_good` fails with: `Err("the home line's nonce account Fcsar... exists and is not an initialized nonce account of ours: an operator looks")` after 5 ticks. It also asserts that `restore_home` keeps the old seed.

**Recommendation.**
- **Open a squatted address in place.** When the address holds a System-owned, data-less account, open it with:
  1. `SystemInstruction::AllocateWithSeed { base: me, seed, space: 80, owner: System }` (index 9; it accepts an account with lamports as long as its data is empty and it is System-owned);
  2. a top-up `Transfer` to the rent-exempt amount;
  3. `InitializeNonceAccount`.

  All of this is still signed by `me` alone.
- **Additionally,** let the operator bump a nonce-seed suffix while the line is idle, since the seed is not identity in any meaningful sense.

---

### A4-03: The fallback line stalls after 64 expiries (Medium)

**Files:**
- `src/solsend.rs:366-374` (each re-sign pushes onto `history`)
- `src/solrpc.rs:37-38,532-535` (`MAX_SIGS = 64`, `statuses` refuses more)
- `src/solsend.rs:350` (`statuses(ps, &s.history)` on every tick)

**Description.** Under `RecentBlockhash`, every proven expiry re-signs the same intent and appends a signature to `history`. `tick` asks for all of `history` in one `getSignatureStatuses` call. Once `history` has 65 entries, `statuses` returns `Refused::Failed("65 signatures per getSignatureStatuses (1..=64)")` before anything else, on every tick, forever. No incident is set, so `sol_clear_incident` cannot free the line; only a code upgrade can.

64 expiries is plausible. The fallback leaves about 40-60 s to land (Q3 §4.7), the canister sets no priority fee (A4-16), and congestion is exactly when expiries cluster. Each re-sign also costs about 26 B cycles.

**PoC.** `poc_a4_03_sixty_four_expiries_stall_the_fallback_line` fails with "after 64 expiries (65 signatures in history) the line is refused every tick".

**Recommendation.**
- A signature proven expired and unknown can never land later, so move it from `history` to an `expired` list. That list stays in `sol_seen` but out of the status query.
- Otherwise, query in chunks of `MAX_SIGS`.
- Cap re-signs per intent and raise an operator-visible `line.error` past the cap.

---

### A4-04: The fallback's "can never land" can read statuses older than its height read (Medium)

**Files:**
- `src/solsend.rs:149-156` (`RecentBlockhash::can_never_land`)
- `src/solsend.rs:117-119` (`none_known`)
- `src/solrpc.rs:153` (`context.slot` dropped)
- `src/solrpc.rs:532-537` (no freshness bound on statuses)

**Description.** The ordering rule (lifetime fact first, statuses second; Q3 §4.3) assumes each provider's view only moves forward between the two reads. Real providers are load-balanced: the rehearsal's run 3 lost a landed `back` to exactly this. `getBlockHeight` may come from an up-to-date backend and `getSignatureStatuses` from one a few slots behind. Nothing asks for a `minContextSlot`, and the transform throws `context.slot` away. A transaction that landed in the last blocks of its validity can therefore look "expired and unknown" to both providers at once. The intent is then re-signed on a fresh blockhash, and both land.

Under the durable nonce, the same stale read only produces a false INCIDENT (liveness, not safety).

**PoC.** `poc_a4_04_a_lagging_status_backend_makes_the_fallback_re_sign_a_landed_intent` (both providers' status reads served stale for one tick) fails with "one intent, 2 successful landings after a stale status read".

**Recommendation.** Bind the second read to the first:
- Keep the height read's `context.slot`, and refuse a status answer whose `context.slot` is lower. The transform can emit `fresh: context.slot >= min`, with `min` carried in the transform context's bucket field, so replicas still agree.
- Or confirm "unknown" with a finalized `getSignaturesForAddress(me)` (`minContextSlot` = the height read's slot) covering the validity window.
- Or wait a safety margin of finalized blocks past `last_valid_height` before trusting "unknown".

---

### A4-05: Decoders do not check account length (Low)

**Files:** `src/solread.rs:91-501` (`Registry`, `Listing`, `Module`, `Wrapper`, `Item`, `Ticket`, `Desk`, `Request`, `Held` `decode`)

**Description.**
- Each decoder checks the owner (`owned`, at the callers) and the Anchor discriminator, then reads its fields and ignores any trailing bytes.
- Anchor allocates exactly `8 + INIT_SPACE` (the `*_BYTES` constants), but nothing compares `data.len()` with them.
- If an upgrade of rook_core (upgradeable until it is frozen) reorders or resizes fields while keeping the discriminator, the canister misreads every field after the change, silently.
- Today the decoders match the program. I checked them field by field against `solana/programs/rook_core/src/state/*.rs`, and `tests/fixtures/solana/rook_core.json` is byte-identical to `solana/target/idl/rook_core.json`.

**Recommendation.** Require `data.len() == *_BYTES` (refusing anything else loudly), as `BackingVault::decode` and `dbc_migrated` already do.

### A4-06: Cost amplification of the sliced `getProgramAccounts` (Low)

**Files:** `src/solrpc.rs:584-592`

**Description.**
- Any `Refused::Failed` on the 64-entry read triggers a second read at the full cap of about 0.9 MB per provider (≈ 9.6 B cycles per provider, by the code's own comment).
- One provider answering more than about 16 kB (which the IC refuses) forces this on every call, and so does an honest outage of one provider.

**Recommendation.** Re-ask at the full cap only when the failure is the IC's size rejection from every provider (`evmread::SIZE_REJECT`). Rate-limit it per collection.

### A4-07: The signing guard checks signers and size only (Informational)

**Files:** `src/solkey.rs:64-77`, `src/solsend.rs:226-241`

**Description.**
- `signable` guarantees that the canister signs alone, as fee payer, a compiled message (never raw bytes) under 1,232 bytes. Nothing else reaches `sign_with_schnorr(ed25519)`.
- It does not constrain the content. Any instruction a flow hands in is signed, including System `Transfer` out of `me`, `AuthorizeNonceAccount` or `WithdrawNonceAccount` on the line's nonce, or any program.
- Today every instruction comes from `solix`, `soltx` or `relaysol`, so this is defence in depth.

**Recommendation.**
- An allowlist of invoked programs: System, ComputeBudget, rook_core, Relay's depository, Token.
- A denylist of System instructions on the nonce account other than `AdvanceNonceAccount` in position 0.

### A4-08: Keyed provider URLs (Informational)

**Files:** `src/solrpc.rs:649-668`, `src/canister.rs:955-962`

**Description.**
- `IcSol::name` keeps only the host for logs.
- The full URL, including any API key, still goes to every replica of a replicated outcall (13 node operators), and it is in the install/upgrade argument.
- `outcall_error` formats the IC's reject message verbatim. Depending on the adapter's wording it may echo the URL into errors that reach the public `log()` query.

**Recommendation.**
- Use header-based keys where the provider allows it.
- Scrub `?…` and path segments from reject texts.
- Treat keys as known to node operators (rotate them, scope them to the canister's needs).

### A4-09: compact-u16 encoding edge cases (Informational)

**Files:** `src/soltx.rs:359-385`

**Description.**
- `compact_u16` casts `n as u16`, which silently truncates above 65,535.
- `read_compact` accepts non-canonical encodings (`0x80 0x00`) and values up to 2²¹; Solana rejects both.
- No production path is affected: the size check caps every field, and `Parts::parse` and `split_tx` are used only by the tests and the fakes.

**Recommendation.** Use `u16::try_from` with an error, and enforce canonical encoding before the parser is used on untrusted bytes.

### A4-10: The v0 / ALT compiler is unused (Informational)

**Files:** `src/soltx.rs:605-646`

**Description.**
- `compile_v0` takes `LookupTable { key, addresses }` from its caller and trusts it.
- No production path calls it, and no lookup-table reader exists.

**Recommendation.** Before it is ever used, the canister must:
- read the table itself at `finalized` and check its owner (the ALT program) and the 56-byte header;
- refuse a deactivated table (`deactivation_slot != u64::MAX`);
- index only addresses extended before the current slot (`last_extended_slot`).

### A4-11: `decode_nonce` ignores the version (Informational)

**Files:** `src/soltx.rs:705-719`

**Description.** Bytes 0..4 (`Versions::Legacy` = 0, `Current` = 1) are not checked. The only nonce read is one the canister created (Current), or one at its own seed address (see A4-02), so this has no effect today.

**Recommendation.** Require `Current`.

### A4-12: The fallback's expiry margin is exactly zero (Informational)

**Files:** `src/solsend.rs:28-29,147,149-156`

**Description.**
- The bank checks a blockhash as `last_hash_index - hash_index <= 150` against the parent's queue, so a transaction can land in a block at height `H + 151`.
- The code expires at `H + 150`. It is safe only because the finalized height is rounded down: `h_rounded > H + 150` implies that the block at `H + 151` is finalized and visible to the status read.
- Any change to the rounding, or to which height is read, would open a one-block window.

**Recommendation.** Set `last_valid_height = H + 151`, plus a margin.

### A4-13: `restore_home` ignores changed identity fields silently (Informational)

**Files:** `src/canister.rs:176-186`, `src/canister.rs:653-656`

**Description.** An upgrade whose `home` names another cluster, `rook_core` or `nonce_seed` is accepted:
- `validate(&arg)` passes on the argument's values;
- the saved identity is kept without a word;
- providers for the new cluster are kept alongside the old cluster name.

The tick's genesis check (`canister.rs:1360-1372`) then refuses everything. That is safe, but confusing.

**Recommendation.** Trap when an identity field in the argument differs from the saved one, and validate the kept `Home` rather than the argument.

### A4-14: `nonce_account` can be cleared by a transient `sol_me` failure (Informational)

**Files:** `src/solhome.rs:487,509-515`

**Description.** If `sol_me()` fails in the tick where the opening lands, the line writes `nonce_account = None`. The next `ready()` adopts the existing account through `open_line`, so it self-heals at the cost of one more read.

**Recommendation.** Derive the address from the opening's own instructions, which are kept in `Saved.instructions`.

### A4-15: The outcome window and "no outcome means not done" (Informational)

**Files:** `src/solhome.rs:189-226`; its consumers are `ethflow.rs:6466-6476`, `frogsol.rs:335-347` and `redeem.rs:1361`.

**Description.**
- `SolLine` keeps 16 outcomes, evicting the oldest regardless of owner.
- A flow that finds its step neither pending nor among the outcomes takes it back as **not done**.
- Eviction is unreachable today, because each owner has at most one step out and takes its outcome at its next pass.
- The rule would turn any future path that loses an outcome (more owners, a batch, the B1→B2 `migrate` of a line whose step already landed) into a re-execution.

**Recommendation.**
- Evict per owner.
- When an outcome is missing, settle the step from the `done` list (signature and error) instead of assuming "not done".

### A4-16: No compute-unit price (Informational)

**Files:** `src/soltx.rs:202-214` (builders exist), unused by `solsend`

**Description.** Without a priority fee, nonce-mode steps can sit pending through congestion (harmless: they are resent). Fallback-mode steps expire repeatedly, which feeds A4-03.

**Recommendation.** Add `SetComputeUnitPrice` and `SetComputeUnitLimit` after the nonce advance. Size the price from `getRecentPrioritizationFees`, with agreement and a cap.

## 3. Checked and found sound

**Signing (`solkey`)**
- One path reaches the Ed25519 key: `sign_message` behind `signable`.
- The message must have exactly one signer, `me`, at index 0 and as fee payer, and must be ≤ 1,232 bytes once signed. Both checks happen before simulation and before signing, and `assemble` re-checks the size.
- The fee payer is always `me` (`compile_parts(.., me, ..)`).
- The BIP340 and secp256k1 keys use other algorithms and paths, so there is no cross-key confusion.
- `sol_address` is operator-only.

**Message building (`soltx`)**
- The legacy compiler orders keys as the SDK does: payer, writable signers, read-only signers, writable non-signers, read-only non-signers, each class sorted by key. Flags are OR-ed and keys de-duplicated.
- The header counts are `u8`-checked and the program indices are correct.
- The nonce advance is always the first instruction (`prepare` prepends the lifetime's prefix).

**System and Anchor encodings**
- The System instructions (`Transfer` 2, `CreateAccountWithSeed` 3, `AdvanceNonceAccount` 4, `InitializeNonceAccount` 6) match their bincode layouts and account metas.
- The compute-budget tags are correct.
- `address_with_seed`, `create_program_address` and `find_program_address` match Solana: seed and seed-count limits, bump descent, and the off-curve check through `CompressedEdwardsY::decompress`, which is the same call as `bytes_are_curve_point`.
- The ATA derivation is correct.
- base58 is correct for leading zeros; `pubkey()` and `signature()` enforce 32 and 64 bytes.

**rook_core instructions (`solix`)**
- Every account list (names, order, signer and writable flags) matches the `#[derive(Accounts)]` structs in `instructions/{floor,wrap,wake}.rs`: `PostFloor`, `Advance`, `Settle`, `ReturnAdvance`, `FundReserve`, `MintItem`, `CustodianTicket`, `RecordDelivery` and `MarkServed`.
- Every PDA is derived from `seeds.rs`'s seeds and never taken from an answer.
- The Anchor discriminators are `sha256("global:<name>")[..8]`, and `MintItemArgs` is Borsh in field order.

**Decoders (`solread`)**
- Field order and sizes match `state/registry.rs`, `wrap.rs`, `floor.rs` and `wake.rs`, enum variant order included (`ListingKind`, `Stage`, `Destination`, `BitcoinNetwork`).
- Every read is bounded: no panic on short or hostile data, strings and vectors are capped at the program's `max_len`, and bools must be 0 or 1.
- `owned()` is applied by every caller (`ethflow::decoded`, `frogsol::decoded`, the `Held` and `Ticket` reads).

**RPC consensus (`solrpc`)**
- At least two providers are required.
- Every provider must answer, and the transformed answers must be byte-identical; anything else is refused and never guessed.
- The transform drops only volatile fields (`context`, `id`, `apiVersion`, `confirmations`, `rentEpoch`, `space`, logs and units). Slots and heights are rounded down.
- RPC errors become agreed classes, and typed readers refuse them.
- Every account, slot, height, block, balance, rent, signature-list, transaction and simulation read is at `finalized`. Statuses use `searchTransactionHistory` and are judged by level.
- A response over `max_response_bytes` is refused by the IC, never truncated, so there is no partial parse.
- JSON robustness:
  - numbers above `u64` give `as_u64() == None`, which is refused;
  - missing fields are refused;
  - base64 errors are refused;
  - an unexpected `data` encoding is kept whole, so it disagrees or fails;
  - duplicate keys can only collapse to one value on both sides.
- Simulation is trusted only to refuse signing. It never authorises anything; the providers must agree that it succeeds.

**The send line (`solsend`)**
- `Saved` is written and committed before the first send (`sign` and `open_line` save before `sol_tick`'s outcall).
- Resends are always the same bytes.
- One transaction is in flight per line (`prepare` refuses while `saved` exists; `busy()` covers pending and opening).
- What landed is read only from agreed statuses.
- "Can never land" reads the lifetime fact first, with the nonce at finalized, apart from A4-04.
- A durable-nonce move under a pending transaction is an incident and is never re-signed.
- A fallback re-sign keeps the intent and adds to `history`.
- A first send answered `blockhash_not_found` counts as sent, so expired fallback bytes are not parked in Preflight.
- `Refused` changes nothing.

**Persistence**
- `Line`, `Saved`, `SolLine` and `LinePending` are the fourteenth element of the upgrade tuple, and `Line.error` has `serde(default)`.
- `migrate` moves B1's line once, refuses when both lines carry state, and keeps B1's pending step on the Ethereum side.
- An upgrade cannot happen mid-await (stop waits).

**Turn queue**
- `claim` serves flows in first-asked order, drops a stale turn (1 h), and never lets a flow take another's outcome (`take` partitions by owner).
- Within a tick no two flows run concurrently, and `sol_clear_incident` is the only writer of `LINE` outside a tick: it is operator-only and refused while paused or mid-tick.

**Init**
- The cluster is on an allowlist, there are at least two distinct providers, and `rook_core` is valid base58 of 32 bytes.
- `nonce_seed` is 1-32 printable ASCII bytes.
- The genesis-hash guard checks the providers against the installed cluster on every tick.
- `restore_home` keeps the identity (A4-13 aside), and the B1→B2 and EVM→Solana switch preconditions are enforced at `post_upgrade`.

**Cycles**
- Every loop is bounded: at most 20 skipped slots, `MAX_KEYS`, `MAX_SIGS`, `POOLS_READ`, `VAULTS_LISTED`.
- Every outcall has an explicit response cap.

## 4. Test results

`cargo test --locked --no-fail-fast` in `autobuyer/canister-rs` at `f2c24e0` plus `tests/audit_a4_line.rs`:

- **All existing tests:** 940 passed.
- **`tests/audit_a4_line.rs`:** 6 tests, all failing by design. Each asserts the property the code should hold, and will pass once the finding is fixed.

| Test | Finding | Result |
|---|---|---|
| `poc_a4_01_abandoned_step_lands_and_its_replacement_lands_too_durable_nonce` | A4-01 | FAILED: 2 successful landings for one intent |
| `poc_a4_01_abandoned_step_lands_and_its_replacement_lands_too_blockhash_fallback` | A4-01 | FAILED: 2 successful landings for one intent |
| `poc_a4_01_anyone_turns_an_abandoned_step_into_an_incident` | A4-01 | FAILED: incident raised by a stranger |
| `poc_a4_02_a_transfer_to_the_nonce_address_blocks_the_durable_nonce_line_for_good` | A4-02 | FAILED: line never opens |
| `poc_a4_03_sixty_four_expiries_stall_the_fallback_line` | A4-03 | FAILED: refused at 65 signatures |
| `poc_a4_04_a_lagging_status_backend_makes_the_fallback_re_sign_a_landed_intent` | A4-04 | FAILED: 2 successful landings for one intent |

The suite also showed 4 failures from another auditor's new file (`tests/audit_a5_sol_flows.rs`), which is not part of this report.

The PoCs use the existing fake pair (`tests/fakerpc/sol.rs`), which runs the canister's own transform. Where a finding needs it, a thin wrapper does one of two things: it forges the answer of a non-replicated send after really forwarding it, or it serves the status read from a lagging backend.

## Verification (independent, 2026-10-10)

The verifier tried to disprove each finding. It re-read the cited code at `f2c24e0`, re-ran `cargo test --locked --test audit_a4_line` (6 of 6 fail as described, with the messages quoted in §4), checked the fakes against the code paths they exercise, and looked for guards elsewhere: in the flows that consume `LineOutcome`, in rook_core's own state checks, and in Solana and IC semantics. No source file was modified.

| ID | Verdict | Final severity | Reason |
|---|---|---|---|
| A4-01 | CONFIRMED-ADJUSTED | **Medium** | The mechanism is real: `preflight_refused` decides on unagreed answers, which contradicts `solrpc::send`'s own doc ("nothing is ever decided on them"); `sol_line` is an unguarded query that returns `Saved.tx`; abandoned bytes stay valid and leave `history`. The impact is narrower than stated. rook_core steps cannot double-execute: `advance` requires `transit_units == 0`, and the flow adopts an open transit (`NeedAdvance` → `transit_open` → `adopt_advance`). Only Relay `deposit_native` (ethflow Deposit, frog Convert) can run twice, and both payouts go to our own addresses, so the result is misallocation rather than loss. The forging vector needs the replica(s) running four separate non-replicated outcalls over two ticks to lie. The no-node INCIDENT vector needs a genuine double preflight refusal, which is a rare race |
| A4-02 | CONFIRMED-ADJUSTED | **Medium** (launch-blocking) | Confirmed: `open_line` refuses any existing non-nonce account. The System Program's `create_account` returns `AccountAlreadyInUse` when `lamports > 0`. `me` is derivable by anyone (`schnorr_public_key` with a `canister_id`), and the seed is in the timelock-published install argument (and in the monitor fixture `rook-home-0`). The impact is availability only, and it exists only before the line first opens: nothing is in flight then, and no funds are at risk. A supported escape exists (a 72 h timelocked upgrade to `blockhash_fallback`). The fix is cheap and must land before launch. Its recommendation (`AllocateWithSeed` + top-up `Transfer` + `InitializeNonceAccount`) is correct and complete, because an outsider can only transfer to a seed address, never allocate or assign it |
| A4-03 | CONFIRMED-ADJUSTED | **Low** (Medium if the fallback becomes the operating mode) | Confirmed: `statuses` refuses more than 64 signatures before any I/O, every tick, with no incident. It is reachable only under the non-default fallback, after 64 consecutive unlanded expiries of one intent (more than an hour without a single landing) |
| A4-04 | CONFIRMED-ADJUSTED | **Low** | The mechanism is real: no `minContextSlot`, and `context` is dropped. Exploiting it needs both providers' status backends to lag at least ~32 slots (behind the other backend's finalized height) at the decisive tick, under the non-default fallback, and only a Relay deposit is harmed (see A4-01). The cited rehearsal evidence (run 3) is a different defect: the tool polled without `searchTransactionHistory` |
| A4-05 | CONFIRMED | Low | Decoders read fixed offsets and ignore `data.len()` |
| A4-06 | CONFIRMED | Low | `program_slices` re-asks at the full cap on any `Refused::Failed` |
| A4-07 | CONFIRMED | Info | `signable` checks signers and size only |
| A4-08 | CONFIRMED | Info | As stated |
| A4-09 | CONFIRMED | Info | `compact_u16` casts `as u16`; `read_compact` is lax and is used only by tests and fakes |
| A4-10 | CONFIRMED | Info | Unused today |
| A4-11 | CONFIRMED | Info | `decode_nonce` checks the length and `state == 1`, not bytes 0..4 |
| A4-12 | CONFIRMED | Info | Safe only through rounding, as stated |
| A4-13 | CONFIRMED | Info | `validate(&arg)` runs before `restore_init` |
| A4-14 | CONFIRMED | Info | `me.and_then(..)` writes `None`; it self-heals through `open_line` adoption |
| A4-15 | CONFIRMED | Info | As stated |
| A4-16 | CONFIRMED | Info | No `SetComputeUnitPrice` in `solsend` |

**Notes**

- **A4-01.**
  - Severity drops from High because:
    - the "double execution" of the PoC (a plain transfer) applies to no rook_core step;
    - a double Relay deposit pays out to our own addresses;
    - the forging vector needs control of the replica(s) running both providers' sends on two consecutive ticks (the IC picks the node for a non-replicated outcall; it is not attacker-chosen).
  - What remains is anyone-triggerable: an INCIDENT (a halt of every Solana flow) once a genuine preflight refusal has abandoned public bytes, and a possible double Relay deposit. It is still a must-fix, because the fix is small and restores the stated invariant.
  - A rook_core step that is abandoned but did land can leave its flow stuck. Example: a `Settling` buy re-signs `settle_advance`, the simulation refuses with `NoAdvance`, and `line.error` is set on every tick. `home_effect` is not consulted on the `ok: None` path (`resolve` calls `mined_home(step, false)` directly).
- **A4-02.** The squat must happen before the first `ready()` on a durable-nonce home. Once our nonce account exists, an outsider can no longer affect it.
- **A4-04.** The same stale-read mechanism under the durable nonce produces a false INCIDENT. That is liveness only, unless the operator clears it (see NEW-A4-01).
- **Fakes.** `tests/fakerpc/sol.rs` advances the nonce on a failed nonced landing and runs the canister's own transform. The `Wrap` forger really forwards before it forges. All of this is faithful to Solana and to the code paths exercised.

**New issues**

| ID | Severity | Issue | Fix |
|---|---|---|---|
| NEW-A4-01 | Low | `sol_clear_incident` → `clear_incident` drops `saved` and its `history` without re-checking the chain. A false incident (A4-04's stale status read under the durable nonce, where the transaction did land) that the operator clears makes the flow take the step back as "never landed" (`mined_home(step, false)`) and re-sign it. For a Relay deposit, that is a second deposit. | Before abandoning, `clear_incident` re-reads the agreed statuses of `history` and refuses if any signature is known. Move the signatures to `done` so `sol_seen` keeps them. |

## Fixes (2026-10-10)

Working tree on `golive-2026-10-08` (uncommitted). Paths are under `autobuyer/canister-rs/`. `cargo test --locked`: 967 passed, 0 failed. That count is the 940 baseline tests, the 10 PoCs (6 here, 4 in A5), 16 new tests and 1 for the rook_core A1-01 mirror. Wasm: 1,691,882 bytes gzipped, under the 2 MiB install limit.

| ID | Status | Fix | Test |
|---|---|---|---|
| A4-01 (Medium) | Fixed | `src/solsend.rs:405` `tick`. A first send counts as sent whatever its non-agreed answers say. A preflight refusal (all providers) sets only `Line.error`, plus `Saved.giving_up` when the refusal is not for lack of funds. A given-up intent is never resent as itself. Durable nonce (`:471`): a burn is signed on the same nonce (an `AdvanceNonceAccount`-only transaction, not simulated) and its signature joins `history`. `Step::Abandoned` (outcome `ok: None`) is reported only when the burn finalizes; if an original signature finalizes first, that landing is the outcome. Fallback (`:449`): abandoned only when `can_never_land` is proven. `abandon_refused` is removed. `Done.abandoned` keeps every signature for `sol_seen`. An `insufficient_funds` refusal is resent with preflight each tick and cleared once accepted. `sol_line` no longer exposes the signed bytes (`src/solhome.rs:331` `public_view`, which empties `saved.tx`); the signature stays visible. `tick_line` (`src/solhome.rs`) maps `Abandoned` to an outcome and `Burning`/`Preflight` to log lines only. Monitor alert text updated (`monitor/src/ethnft.mjs:111`). | `audit_a4_line.rs`: the three `poc_a4_01_*` (assertions flipped only where they asserted the old "abandoned" outcome or re-signed without one; every property assertion kept); `fix_a4_01_a_given_up_step_is_reported_unlanded_only_when_its_nonce_burn_lands`; `fix_a4_01_under_the_fallback_a_given_up_step_is_reported_unlanded_only_after_its_proven_expiry`; `ethnft_sol.rs` incident test (the public view hides the bytes) |
| A4-02 (Medium) | Fixed | `src/solhome.rs:424` `open_line`. A System-owned, data-less, non-executable account at the nonce address is opened in place with `solsend::open_squatted_nonce_ixs` (`src/solsend.rs:237`): `AllocateWithSeed(80, System)` (`src/soltx.rs:194`), a top-up `Transfer` to the rent-exempt amount when needed, then `InitializeNonceAccount`, all signed by `me` alone. Any other account, including a nonce with a foreign authority, is still refused for the operator. The fake System Program executes index 9. | `poc_a4_02_*`; `fix_a4_02_the_in_place_opening_is_allocate_top_up_initialize`; `ethnft_sol.rs` `fix_a4_02_a_squatted_nonce_address_is_opened_in_place` (executed: `system:9, system:2, system:6`, then the advance; a foreign-authority nonce is still refused) |
| A4-03 (Low) | Fixed | On a fallback re-sign, the proven-expired signatures move from `history` to `Saved.expired` (at most `EXPIRED_KEPT` = 64, `src/solsend.rs:393`). They stay in `sol_seen` and `Done.history`. `solrpc::statuses_at` (`src/solrpc.rs:542`) also asks in chunks of `MAX_SIGS`. | `poc_a4_03_*` (70 expiries); `tests/solsend.rs` re-sign test updated (`history` = the new signature, `expired` = the old one) |
| A4-04 (Low) | Fixed | `none_known` (`src/solsend.rs:136`) reads a finalized slot after the lifetime fact and binds the status read to it (`statuses_at(.., min_slot)`). The transform (`src/solrpc.rs:170`) answers `{"stale": true}` when the provider's `context.slot` is below the bucket, so replicas still agree. A stale answer means "not proven", never "unknown", under both lifetimes, which also removes the false-INCIDENT variant. | `poc_a4_04_*`; `fix_a4_04_a_status_answer_older_than_the_bound_slot_is_stale` |
| A4-05 (Low) | Fixed (done last, after the program agent finished) | `src/solread.rs:49` `exact`. Registry, Listing, Module, Wrapper, Item, Ticket, Desk, Request and HeldMarker must be exactly `*_BYTES` long; anything else is refused loudly. | the whole suite: every fake account is exact |
| NEW-A4-01 (Low) | Fixed | `sol_clear_incident` (`src/solhome.rs:891`) is now async. It holds the tick flag (`hold_tick`), re-reads the agreed statuses of every signature of the pending intent (expired ones included), and refuses if any is known or the line changed meanwhile. `solsend::clear_incident` (`src/solsend.rs:515`) moves the intent to `done` as abandoned, so `sol_seen` keeps its signatures. | `ethnft_sol.rs` `sol_a_nonce_moved_under_a_pending_step_*` (`clear_incident(.., true)` is refused and nothing is dropped; `done` keeps the history) |
| A4-06…A4-16 (Low/Info) | Not changed | Out of the requested scope. A4-14 still self-heals; A4-15 is still unreachable (one step per owner). | |

**Also done here.** rook_core's A1-01 change is mirrored in `Listing::keeper_down` (`src/solread.rs:620`): an Inscriptions listing's own `poster_paused` now puts its keeper down. Test: `an_inscriptions_listings_poster_switch_puts_its_keeper_down`. The canister does not decode Treasury, so its new `round_started_at` field does not affect it.

**State and interface changes** (all `opt`, backward compatible):
- `Saved.giving_up`, `Saved.burn` and `Saved.expired`; `Done.abandoned`; `SolLine.relay` (A5-03).
- The same changes are in `etch_autobuyer.did`.
- The B1 `EthState.sol` line embeds these types, so the EthState encoding fixtures were regenerated: `eth-state-round{2,4,5,6,7,8-b8l1}.hex`, `eth-state-dormant{,-served}.hex` and `sol-line-incident.hex` (now the public view).
- The monitor's 273 tests pass on them.
