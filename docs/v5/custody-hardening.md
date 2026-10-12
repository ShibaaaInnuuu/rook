# Custody hardening: what to change, in order, and the wiring for what is built

Written 2026-09-23 on branch `custody-security`. It answers `docs/v5/custody-threat-model.md` (T1–T16). Every proposal below
names the threat it closes, the file where the change goes, and who owns that file. That branch edited **no** shared file: not
`canister.rs`, `lib.rs`, `looper.rs`, `keeper/`, `web/` or any Solidity. The three proposals that need new logic (H2, H3, H4)
were built as standalone pure modules with tests and **no wiring**, and their wiring was written out below as diffs.

**Wired on branch `wire-hardening`** (2026-09-23): H1 at `97178ad`; H2, H3, H4 and H9's ticket read at `2562c0e`. The diffs
below are what was specified; where the wiring differs, an "As wired" note under the section says how and why. The status
column of the table is current.

## Ranked by risk reduction per unit of effort

Effort: XS < 1 hour, S < 1 day, M < 1 week, L > 1 week. Risk reduction is measured against the threat model's "max at risk
today".

| Rank | # | Proposal | Closes | Risk reduction | Effort | Where / owner | Status |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | **H1** | The guardian's pause stops **every** signing, sending and staging entry point, not only `tick` | T6; half of T5 | stolen operator after pause: Σ E_c → 0 | XS (≈ 25 lines) | `canister.rs`, `nftnet.rs`, `frognet.rs`, `btcnet.rs`, `floornet.rs` | **wired** `97178ad` |
| 2 | **H2** | **Custody caps**: per collection (items, value, per-item price) and global. Default-deny for unlisted collections. Raised only by a timelocked upgrade argument, at most ×4 per proposal. Bounds `nft_set_target` | T3, T5, T7, T10 (bounds every one of them) | everything → the stage cap | S to wire | `src/custodycap.rs` + hooks | **wired** `2562c0e` |
| 3 | **H3** | **Outbound rate limits** (items, mints, value per 24 h, per collection and global) with a **tripwire** that freezes custody onto the guardian pause | T10, T1 bugs, T7 at the canister | B in hours → 6 items a day, then frozen | S to wire | `src/outflow.rs` + hooks | **wired** `2562c0e` |
| 4 | **H4** | **Delivery-only-to-ticket-destinations proof**: before anything custody signs is saved, its finalized bytes are re-read and checked against the ticket (or deposit) they serve | T8, T10 (with H9), T16, builder bugs | a mis-sent item → refused before saving | S to wire | `src/outbound.rs` + hooks | **wired** `2562c0e` (Bitcoin; the EVM delivery path is not built yet, C10) |
| 5 | H5 | **Watcher v2**: decode and diff every upgrade argument; poll controllers and module hashes (certified); watch 4663's `Propose*` events; a **canary** on every custody address | T1, T2, T15, T10 detection | detection within one poll | S | `scripts/timelock-watch.mjs` + a new `scripts/custody-canary.mjs` | **built** as `monitor/` on `p1-monitor` (K6, `docs/v6/k6-watcher-canary.md`); the 4663 `Propose*`/`CapPosted` watch and the hourly ledger reconciliation are not in it (§8 there) |
| 6 | H6 | **Upgrade announcement rule**: no proposal without its commit, reproducible hash, argument diff and a public post made at the moment of proposing | T1, T2 | turns 72 h of silence into 72 h of review | XS | process (`docs/v5/launch-gates.md`, G3) | spec |
| 7 | H7 | **Controller = timelock only**, verified by two outsiders; install, handover and blackhole in one witnessed sitting; nothing funded before | T1 (the pre-handover window), T2 | High → Low for T1 | XS | ceremony (`docs/v5/launch-gates.md`, G2) | spec |
| 8 | H8 | **Conflict-out**: a delivery or return whose bytes were ever broadcast and is then refused gets its fee input spent back to custody, so the leaked bytes can never be mined | T8 | 1 item per eviction → 0 | M | `redeem.rs`, `frogflow.rs`, `conflict.rs` (uses H4's `ConflictOut`) | **built** `72531db` |
| 9 | H9 | **Finality for outbound decisions**: tickets, deposits and arrivals are read at each chain's `finalized` block | T9, part of T10 | reorg losses → ≈ 0 | M | `pin.rs`, `custodynet.rs`, `nftnet.rs` | **partly wired** `2562c0e`: the ticket and its `Redeemed` log (what H4 rests on); deposits' EVM side and NFT arrivals still read at the fast pin |
| 10 | H11 | **On-chain mint rate limit** in `Wrapper` and `NftWrapper` (M per 24 h, both clocks, fixed at construction) | T7 | P + Q in one block → M items a day | S | Solidity (contracts agent) | spec |
| 11 | H12 | **Keeper cap step bound**: `postCap` within ±20% of the last fresh cap, at most one post per 10 minutes | T4 | Q at any price → Q at cap × 1.2ⁿ | S | Solidity (contracts agent) | spec |
| 12 | H13 | **Cycles**: automatic top-up, a 90-day freezing threshold, three-level alarms, bounded saved state and a `pre_upgrade` drill | T13 | a frozen custodian → an alarm days ahead | S | ops + `canister.rs` | spec |
| 13 | H10 | **Independent providers** (≥ 3 operators on every chain, one of them our own node) and **separate keys per role** | T10, T16 | collusion needs 3 operators; one role's bug cannot move another role's items | M–L | ops; key paths need `NftWrapper` to name the holder apart from the minter | spec |
| 14 | H14 | **Frontend integrity**: published build hashes and an IPFS copy, registrar lock and DNSSEC, strict CSP, destination confirmation | T14 | shortens and exposes a compromise | M | `web/` (web agent) | spec |

What "stage cap" means is in `docs/v5/launch-gates.md`: stage 0 is 3 items a collection and 10 ETH overall; each stage is ×4 the
last.

---

## H1. The pause stops every signing path

**Why first.** It costs one function and one line in each of about twelve methods. It closes a whole threat (T6). It also makes
the key-ceremony runbook's answer to a stolen operator ("a guardian pauses the buyer") true.

**Spec.** A new guard `not_paused()` is called right after `only_operator()` in every update method that signs, sends, records
a quote or stages a spend. Methods that only read, and `abandon` (whose only effect is returning ETH to the vault), stay open.
`set_paused` is unchanged. A test reads the source of every gated method and fails if the call is missing, in the style of the
existing caller-guard test.

**The diff** (against `9022920`):

```diff
--- a/autobuyer/canister-rs/src/canister.rs
+++ b/autobuyer/canister-rs/src/canister.rs
@@ fn may_operate(is_controller: bool, caller: candid::Principal, operator: Option<candid::Principal>) -> bool {
     is_controller || operator == Some(caller)
 }
 
+/// Refuses while the buyer is paused (docs/v5/custody-hardening.md, H1). The pause used to be read only inside `tick`, so a
+/// stolen operator key could still sign a purchase or a Seaport fill after a guardian had paused the buyer.
+pub(crate) fn not_paused() -> Result<(), String> {
+    if PAUSED.with(|p| *p.borrow()) {
+        return Err("refused: the buyer is paused; k guardians resume it through the timelock".into());
+    }
+    Ok(())
+}
+
@@ pub async fn open_purchase(amount_wei: candid::Nat, rune: String, units: candid::Nat, recipient_script_hex: String,
     cap_ref: String) -> Result<u64, String> {
     only_operator();
+    not_paused()?;
@@ pub fn relay_venue_quote(purchase_id: u64, quote: Quote) -> Result<String, String> {
     only_operator();
+    not_paused()?;
@@ pub fn relay_venue_psbt(purchase_id: u64, psbt_hex: String, to_sign: Vec<u32>) -> Result<String, String> {
     only_operator();
+    not_paused()?;
@@ pub async fn sign_purchase(purchase_id: u64, psbt_hex: String, input_index: u32) -> Result<SignedInput, String> {
     only_operator();
+    not_paused()?;
@@ pub async fn record_quote(purchase_id: u64, quote: Quote) -> Result<String, String> {
     only_operator();
+    not_paused()?;
@@ pub async fn rehearse_sign_sighash(network: String, sighash_hex: String) -> Result<String, String> {
     only_operator();
+    not_paused()?;
@@ pub async fn rehearse_sign_evm(chain_id: u64, hash_hex: String) -> Result<EvmSignature, String> {
     only_operator();
+    not_paused()?;
@@ pub async fn rehearse_broadcast_evm(chain_id: u64, raw_hex: String) -> Result<EvmSend, String> {
     only_operator();
+    not_paused()?;
--- a/autobuyer/canister-rs/src/btcnet.rs
+++ b/autobuyer/canister-rs/src/btcnet.rs
@@ pub async fn rehearse_broadcast_btc(network: String, raw_hex: String) -> Result<BtcSend, String> {
     only_operator();
+    crate::canister::not_paused()?;
--- a/autobuyer/canister-rs/src/nftnet.rs
+++ b/autobuyer/canister-rs/src/nftnet.rs
@@ pub fn nft_set_target(chain_id: u64, collection: String, token_id: String, cap_wei: candid::Nat) -> Result<String, String> {
     only_operator();
+    crate::canister::not_paused()?;
@@ pub async fn nft_sign(chain_id: u64, collection: String, token_id: String, order_hex: String) -> Result<NftPlan, String> {
     only_operator();
+    crate::canister::not_paused()?;
@@ pub async fn nft_send(chain_id: u64) -> Result<EvmSend, String> {
     only_operator();
+    crate::canister::not_paused()?;
--- a/autobuyer/canister-rs/src/frognet.rs
+++ b/autobuyer/canister-rs/src/frognet.rs
@@ pub fn frog_offer(frog_id: u64, inscription_id: String, psbt_hex: String, to_sign: Vec<u32>, proof_hex: Vec<String>)
     -> Result<String, String> {
     only_operator();
+    crate::canister::not_paused()?;
@@ pub fn refuse_deposit(outpoint: String, reason: String) -> Result<String, String> {
     only_operator();
+    crate::canister::not_paused()?;
```

`sign_purchase` returns `Result`, so `?` works everywhere listed. `rehearse_open_purchase` and `rehearse_fixtures` only exist on
signet installs. Gate them too, for uniformity.

The test (in `canister.rs`'s tests module):

```rust
#[test]
fn every_method_that_signs_sends_or_stages_a_spend_refuses_while_paused() {
    let files = [include_str!("canister.rs"), include_str!("nftnet.rs"), include_str!("frognet.rs"), include_str!("btcnet.rs")];
    let gated = ["open_purchase", "relay_venue_quote", "relay_venue_psbt", "sign_purchase", "record_quote", "rehearse_sign_sighash",
        "rehearse_sign_evm", "rehearse_broadcast_evm", "rehearse_broadcast_btc", "nft_set_target", "nft_sign", "nft_send",
        "frog_offer", "refuse_deposit"];
    for name in gated {
        let src = files.iter().find(|s| s.contains(&format!("fn {name}("))).unwrap_or_else(|| panic!("{name} not found"));
        let body: String = src.split(&format!("fn {name}(")).nth(1).unwrap().chars().take(500).collect();
        assert!(body.contains("not_paused()?;"), "{name} signs, sends or stages a spend and does not check the pause");
    }
}
```

**Also** (docs): `docs/deploy/key-ceremony.md`'s stolen-operator row should say "and the keeper service holding the operator
identity is stopped". Until H1 lands, the pause alone does not stop the operator (T6).

**As wired** (`97178ad`): `not_paused()` after `only_operator()` in the fourteen methods above, the two rehearsal helpers,
`nft_status`, `frog_proof`, and the floor-feed merge's `floor_nft_sign` and `floor_nft_arrived`. The source test
(`canister.rs`, `every_operator_method_that_acts_refuses_while_paused`) scans every file in `src/` and fails for any operator
method that neither checks the pause nor is on the argued read-only list.

---

## H2. Custody caps (`src/custodycap.rs`, wired at `2562c0e`)

**What it is.** A pure module: `CapPolicy { collections: Vec<CollectionCap>, global_max_items, global_max_value_wei }`, with
`CollectionCap { collection, max_items, max_value_wei, max_item_wei }`. A `Book` of entries `{ id, collection, value_wei,
in_flight }`. And these functions:

| Function | Answers |
| --- | --- |
| `validate(&policy)` | canonical keys (`btc:0x<wrapper>`, `<chain id>:0x<collection>`, lower-case), no duplicates, per-item ≤ collection value, collection ≤ global |
| `validate_change(&old, &new)` | lowering is always allowed; a raise is at most ×`MAX_RAISE_FACTOR` (4); a new collection or global cap starts at most at stage 0 (3 items, 10 ETH) |
| `check_target_cap(&policy, collection, cap_wei)` | `nft_set_target` may record this cap: the collection is listed and the cap ≤ its `max_item_wei` |
| `check_acquire(&policy, &book, collection, cost_wei)` | one more item fits under every cap. Unlisted → refused. Overflow → refused, not saturated |
| `reserve` / `settle` / `admit` / `release` | the book's lifecycle: in flight → held (under its own id) or released; a deposit admitted held; an item released when it leaves |
| `Book::headroom(&policy)` | per collection: held, in flight, value, items and value left, for the public page |

**Why the raise limit.** The timelock's stated limit is a proposer and k guardians colluding. The ×4 rule means that even then,
custody's exposure rises by at most four times per 72 public hours. It cannot jump from 10 ETH to the whole market in one
proposal. The book is also bounded by the caps, which keeps the saved state small (T13).

**Tests** (`src/custodycap.rs`, 10; `tests/hardening.rs`): every cap binds at its edge and not before; the global item and value
caps bind across collections; in-flight counts before it lands, and a failed purchase frees its room; an id is counted once; a
deposit counts like a purchase; raises are ≤ ×4 and new caps start at stage 0; overflow is refused; and the operator's NFT drain
(T5) is refused at the target and at the cap.

**The wiring** (canister agent; about 60 lines):

```diff
--- a/autobuyer/canister-rs/src/lib.rs
+++ b/autobuyer/canister-rs/src/lib.rs
@@ pub mod frognet;
+// custody hardening (docs/v5/custody-hardening.md, H2–H4)
+pub mod custodycap;
+pub mod outflow;
+pub mod outbound;
 
 pub mod canister;
--- a/autobuyer/canister-rs/tests/hardening.rs
+++ b/autobuyer/canister-rs/tests/hardening.rs
-#[path = "../src/custodycap.rs"]
-mod custodycap;
-#[path = "../src/outbound.rs"]
-mod outbound;
-#[path = "../src/outflow.rs"]
-mod outflow;
+use etch_autobuyer::{custodycap, outbound, outflow};
--- a/autobuyer/canister-rs/src/canister.rs
+++ b/autobuyer/canister-rs/src/canister.rs
@@ pub struct Init {
     pub evm_chains: Option<Vec<crate::chains::EvmChain>>,
+    /// How much custody may hold (docs/v5/custody-hardening.md, H2). Taken from every upgrade's argument, but only through
+    /// `custodycap::validate_change` against the running policy: a raise is at most ×4 per proposal. `None` is the empty
+    /// policy, which refuses every acquisition.
+    pub custody_caps: Option<crate::custodycap::CapPolicy>,
+    /// Outbound limits (H3). Required once `wrapper` or `evm_chains` is set.
+    pub outflow_limits: Option<crate::outflow::Limits>,
 }
 
 thread_local! {
+    /// What custody holds and is acquiring, as the caps count it (H2), and what left it in the last day (H3).
+    pub(crate) static CUSTODY_BOOK: RefCell<crate::custodycap::Book> = RefCell::new(Default::default());
+    pub(crate) static OUTFLOW: RefCell<crate::outflow::Outflow> = RefCell::new(Default::default());
@@ fn restore_init(saved: Option<Init>, arg: &Init) -> Init {
     kept.evm_chains = arg.evm_chains.clone();
+    kept.custody_caps = arg.custody_caps.clone();     // post_upgrade has checked validate_change(saved, arg) first
+    kept.outflow_limits = arg.outflow_limits.clone();
     kept
 }
@@ fn validate(arg: &Init) {
+    if let Err(e) = crate::custodycap::validate(&arg.custody_caps.clone().unwrap_or_default()) {
+        panic!("refused: custody_caps: {e}");
+    }
+    if arg.wrapper.is_some() || arg.evm_chains.as_ref().is_some_and(|c| !c.is_empty()) {
+        let l = arg.outflow_limits.as_ref().expect("refused: outflow_limits are required once custody exists");
+        if let Err(e) = crate::outflow::validate(l) {
+            panic!("refused: outflow_limits: {e}");
+        }
+        for c in arg.custody_caps.iter().flat_map(|p| p.collections.iter()) {
+            assert!(crate::outflow::exit_fits(c.max_items, l.per_collection_items, ASSUMED_DELAY_SECS),
+                "refused: {} holds up to {} items and lets {} out a day: holders could not leave within the timelock's delay",
+                c.collection, c.max_items, l.per_collection_items);
+        }
+    }
@@
+/// The timelock's floor (`timelock-rs/src/rules.rs`): the least time holders have to leave before an upgrade lands.
+const ASSUMED_DELAY_SECS: u64 = 72 * 3600;
@@ type Saved = (Option<Init>, Vec<Purchase>, bool, Vec<String>, Option<crate::redeem::Custody>, Option<crate::chains::NftState>,
-    Option<crate::frogflow::FrogState>);
+    Option<crate::frogflow::FrogState>, Option<(crate::custodycap::Book, crate::outflow::Outflow)>);
@@ fn pre_upgrade() {
         Some(crate::frognet::FROGS.with(|f| f.borrow().clone())),
+        Some((CUSTODY_BOOK.with(|b| b.borrow().clone()), OUTFLOW.with(|o| o.borrow().clone()))),
     );
@@ fn post_upgrade(arg: Init) {
-        Ok(((saved_init, purchases, paused, log, custody, nft, frogs),)) => {
+        Ok(((saved_init, purchases, paused, log, custody, nft, frogs, guard),)) => {
             validate(&arg);
+            let old = saved_init.as_ref().and_then(|i| i.custody_caps.clone()).unwrap_or_default();
+            if let Err(e) = crate::custodycap::validate_change(&old, &arg.custody_caps.clone().unwrap_or_default()) {
+                ic_cdk::trap(&format!("refused: custody_caps: {e}"));
+            }
             INIT.with(|i| *i.borrow_mut() = Some(restore_init(saved_init, &arg)));
+            let (book, outflow) = guard.unwrap_or_default();
+            CUSTODY_BOOK.with(|b| *b.borrow_mut() = book);
+            OUTFLOW.with(|o| *o.borrow_mut() = outflow);
+
+/// What custody may hold and holds, for anyone (the proof-of-reserves page reads it).
+#[ic_cdk::query]
+pub fn custody_caps() -> (crate::custodycap::CapPolicy, Vec<crate::custodycap::Headroom>) {
+    let p = cfg().custody_caps.unwrap_or_default();
+    (p.clone(), CUSTODY_BOOK.with(|b| b.borrow().headroom(&p)))
+}
```

The hooks in the flows. Each one reads `cfg().custody_caps.unwrap_or_default()` as `policy`, and `CUSTODY_BOOK` as `book`:

| Where | Hook |
| --- | --- |
| `nftnet::nft_set_target`, after parsing | `custodycap::check_target_cap(&policy, &format!("{chain_id}:{}", h0x(&c)), cap)`. Refuse on error. **This closes T5's drain** |
| `nftnet::nft_sign`, after `check` and **before** the signing await | `custodycap::reserve(&policy, &mut book, &format!("nft:{chain_id}:{}", plan.order_hash), &key, plan.total_wei)`. On any later error in the same call, `settle(…, None)` |
| `nftnet::nft_status`, when `chains::settle` returns an outcome | Arrived → `settle(&flight, Some(&format!("{chain}:{collection}:{token_id}")))`; Reverted or Superseded → `settle(&flight, None)` |
| `frogflow`, where a frog purchase leaves `Wanted` (its PSBT judged and signed) | `reserve(…, &format!("frog:{id}"), &format!("btc:0x{wrapper}"), cost_wei)` |
| `frogflow`, frog `Held` / failed | `settle(Some(inscription_id))` / `settle(None)` |
| `frogflow::check_deposit`, before `Ready` | `admit(…, inscription_id, "btc:0x…", mark)`, where `mark` is the sink's fresh `item_cap`, or the collection's `max_item_wei` if none. On `Err`: `DPhase::Refused { reason: Reason::OverCustodyCap }` (returnable: the depositor gets it back, and the page shows the headroom) |
| `redeem` → `Done`; `frogflow` → `Returned` | `custodycap::release(&mut book, inscription_id)` |

**As wired** (`2562c0e`), where it differs from the diff above:
- `Init::custody_caps = None` is **launch stage 0 for the wrapper's inscriptions** (`custodycap::stage0`, `canister::caps_of`),
  not the empty policy: an install with no argument is the canary stage of `docs/v5/launch-gates.md` G7, and still refuses every
  unlisted collection. `init` checks `validate_change(empty, caps)` too, so an install cannot start above stage 0.
- The saved state's new element is the **ninth**, after the floor-feed merge's `Option<FloorState>`:
  `Option<(custodycap::Book, outflow::Outflow)>`. A state saved with eight elements restores
  (`a_state_saved_before_the_custody_guard_existed_still_restores_and_the_book_and_limiter_round_trip`).
- The flows reach the book through one `Io` method, `with_caps(|policy, book| …)`, rather than reading canister state directly,
  so the fake worlds in `tests/redeem.rs` and `tests/frogs.rs` carry their own.
- `nft_set_target`'s check lives in `nftnet::record_target` (pure, tested in `tests/hardening.rs`). NFTs are held under
  `<chain>:0x<collection>:<token id as a 32-byte word>` (`nftnet::nft_item_id`); the floor claim admits the ape under that id if
  `nft_status` had not.
- The frog flow also asks `check_acquire` **before a want starts** (`frogflow::room`, at the sink's and the desk's start and at
  an EVM want's planning), so no purchase is opened that the caps would refuse at the offer.
- `Redemption.inscription` and `DepositRec.admitted` record what the book holds for each, so `release` on `Done` / `LostRace` /
  `Returned` needs no second lookup. Both are `#[serde(default)]` and `opt` in the `.did`.
- A deposit over the cap is `DPhase::Refused { reason: Reason::OverCustodyCap(..) }`, returnable as every refusal is.
- Two public queries: `custody_caps()` (policy and headroom) and `custody_outflow()` (limits and the limiter, freeze included).

---

## H3. Outbound rate limits and the tripwire (`src/outflow.rs`, wired at `2562c0e`)

**What it is.** A pure sliding-window limiter. `Limits { per_collection_items, global_items, per_collection_mints,
global_mints, global_value_wei, trip_factor }`. `Outflow::check(limits, key, collection, flow, amount, now)` answers `Allow`,
`Wait { until }` or `Frozen(cause)`. `Outflow::record(…)` counts an act once it is committed: idempotent per key, so a restart
does not double-count. `Flow` is `Item` (an item leaves), `Mint` (a wrapper unit is minted on our word) or `Value` (wei leaving
to others).

**The tripwire.** When the **distinct** acts asking to go out in one window reach `trip_factor` × a limit, custody freezes. A
waiting ticket asked every tick is one ask, not a run. The wiring maps the freeze onto the buyer's `PAUSED`. So lifting it takes
what lifting a pause takes after the handover: k guardians through `resume_target`, which calls `set_paused(false)`, which calls
`Outflow::unfreeze(true)`. A false positive (a genuine bank run) costs time, not items: deliveries wait, and a ticket that cannot
be delivered inside its margin refunds.

**The bank-run constraint.** A rate limit must never trap holders behind a malicious upgrade they can see coming.
`exit_fits(max_items, per_day, delay)` requires that a collection's whole cap can leave within the timelock's delay less one day.
The wiring refuses an install whose caps and limits break it (H2's diff, `validate`). Stage 1 fits (12 items at 6 a day in 2
days). Stage 3 needs a 7-day timelock (`docs/v5/launch-gates.md`).

**Tests** (`src/outflow.rs`, 10; `tests/hardening.rs`): the seventh delivery of a day waits exactly until the first leaves the
window; collections do not spend each other's room, but share the global limit; value in wei; forged tickets (T10) → 6 delivered,
the 18th distinct ask trips the freeze, and nothing leaves afterwards; one ticket asked every minute for a day never trips; only a
controller unfreezes, and the same demand does not re-trip at once; a manual freeze keeps its first cause; recording is
idempotent and the window bounds the state; the bank-run check.

**The wiring** (canister agent; about 40 lines, on top of H2's `Init`/`Saved`/state changes):

```diff
--- a/autobuyer/canister-rs/src/canister.rs
+++ b/autobuyer/canister-rs/src/canister.rs
@@ pub fn set_paused(p: bool) {
     if p {
         only_operator();
     } else {
         only_controller();
     }
     PAUSED.with(|x| *x.borrow_mut() = p);
+    let now = ic_cdk::api::time() / 1_000_000_000;
+    OUTFLOW.with(|o| {
+        let mut o = o.borrow_mut();
+        if p { o.freeze("paused", now) } else { o.unfreeze(true).expect("a controller") }
+    });
     note(if p { "paused" } else { "resumed" });
 }
+
+/// Ask the limiter; a trip pauses the buyer (H3). Every outbound hook calls this, then `outflow_record` once its bytes are saved.
+pub(crate) fn outflow_check(key: &str, collection: &str, flow: crate::outflow::Flow, amount: u128) -> Result<(), String> {
+    let Some(l) = cfg().outflow_limits else { return Err("refused: no outflow limits configured".into()) };
+    let now = ic_cdk::api::time() / 1_000_000_000;
+    match OUTFLOW.with(|o| o.borrow_mut().check(&l, key, collection, flow, amount, now)) {
+        crate::outflow::Verdict::Allow => Ok(()),
+        crate::outflow::Verdict::Wait { why, .. } => Err(format!("waits (outflow limit): {why}")),
+        crate::outflow::Verdict::Frozen(cause) => {
+            PAUSED.with(|x| *x.borrow_mut() = true);
+            note(format!("CUSTODY FROZEN: {cause:?}"));
+            Err(format!("refused: custody is frozen: {cause:?}"))
+        }
+    }
+}
+
+pub(crate) fn outflow_record(key: &str, collection: &str, flow: crate::outflow::Flow, amount: u128) {
+    let now = ic_cdk::api::time() / 1_000_000_000;
+    OUTFLOW.with(|o| o.borrow_mut().record(key, collection, flow, amount, now));
+}
```

The hooks. `redeem.rs` and `frogflow.rs` reach the world only through their `Io` traits, so each gains two methods:
`outflow_check(&mut self, key, collection, flow, amount) -> Result<(), String>` and `outflow_record(…)`. The canister's
implementations call the functions above. The test fakes (`tests/redeem.rs`, `tests/frogs.rs`) answer `Ok(())` and count the
calls.

| Where | Before | After the bytes are saved |
| --- | --- | --- |
| `redeem::owed`, after step 1 (the rule) and before locating the frog | `io.outflow_check(&format!("ticket:{}", r.ticket), &collection, Flow::Item, 1)?` | step 6, after `io.save(r)`: `io.outflow_record(…)` |
| `frogflow::give_back`, before building | `…("return:{outpoint}", Flow::Item, 1)` | after `d.phase = Returning` is saved |
| `frogflow`, before sending `wrapDeposit` or a sink mint | `…("mint:{outpoint or frog id}", Flow::Mint, 1)` | after the EVM bytes are saved |
| `nftnet::nft_sign`, before the signing await | `…("fill:{order_hash}", "{chain}:{collection}", Flow::Value, total_wei)` | after `in_flight` is saved |
| (spec) NFT mint on 4663 and NFT delivery on chain *c* (`docs/v4/nft-wrapper.md` §5) | `Flow::Mint` / `Flow::Item` | likewise |

`collection` is H2's key for the item's collection (`btc:0x<wrapper>` for frogs).

**As wired** (`2562c0e`), where it differs: `Init::outflow_limits = None` is stage 0 (`outflow::Limits::stage0`), and the limits
are required whatever custody exists (there is always a policy). The keys: deliveries `ticket:<id>`, returns `return:<outpoint>`,
`wrapDeposit` mints `mint:<outpoint>` and `mint:frog:<id>`, floor claims `claim:<want>` (a mint on our word), Seaport fills
`fill:<order hash>` and frog buys `buy:frog:<id>` as `Flow::Value`. A trip sets `PAUSED` once and logs `CUSTODY FROZEN`; the
guardian's pause keeps a trip as the freeze's first cause; only a controller's `set_paused(false)` unfreezes.

---

## H4. Delivery only to the ticket's destination (`src/outbound.rs`, wired at `2562c0e`)

**What it is.** A verifier that re-reads the **finalized signed bytes** of every custody spend, with its own minimal parser (no
shared code with `psbt.rs` or `custody.rs`), and checks them against a `Purpose` stated separately:

- `Delivery { ticket: TicketFacts, item }`. Input 0 is the item, and every other input pays custody. Output 0 pays **the
  ticket's** destination exactly the item's value. At most one more output, and it pays custody. The ticket is `Resolved`, read at
  a finalized block (`require_finalized`), its `Redeemed` event names the same destination (`require_event`), and it names this
  item. The fee is ≤ 25,000 sats. The destination is not custody.
- `DepositReturn { deposit, sender_script }`: the same shape, back to the sender.
- `ConflictOut { spend }`: spends the leaked input, and every output pays custody (H8).
- EVM: `verify_evm(call, NftDelivery { … })` accepts exactly `transferFrom(custodian, destination, tokenId)` to the collection,
  with value 0, on the right chain, for a resolved and finalized ticket. It refuses `safeTransferFrom` (the re-roll of T8),
  `approve` and `setApprovalForAll` by name, and anything else. `NonceBurn` is 0 value to ourselves with no data.

**What it adds over what exists.** `custody::check` verifies the builder's own output against the builder's own inputs. H4
verifies the bytes against the **ticket**, independently. So a destination swapped between read and build, a new code path that
signs with the custody key for an unlisted reason, or change sent to a stranger is refused **before the bytes are saved**, and
saved bytes are the only bytes ever sent. It does not stop a ticket forged consistently at a finalized block. That is H3 and H10.

**Tests** (`src/outbound.rs`, 13; `tests/hardening.rs`): the parser round-trips and refuses truncation, trailing bytes and
non-minimal lengths; an honest delivery passes; each of wrong destination, stranger change, a third output, less than the whole
item, custody as destination, an unresolved, soft, wrong-item or event-disagreeing ticket, a foreign input, mismatched prevouts,
the wrong first input, a high fee and an overspend is refused; returns go only to the sender; a conflict-out must spend the leaked
input and pay only custody; the EVM rules one by one; and the selectors equal their keccak. Against the **real builder**: a
delivery made by `custody::build` + `psbt::finalize` for inscription 0's real outpoint passes when the ticket names its
destination, and the same builder's output for another address is refused against the ticket.

**The wiring** (canister agent; about 30 lines):

```diff
--- a/autobuyer/canister-rs/src/redeem.rs
+++ b/autobuyer/canister-rs/src/redeem.rs
@@ pub trait CustodyIo {
+    /// The ticket as `outbound` needs it: read at the chain's finalized block, with its `Redeemed` log (H9).
+    async fn ticket_facts(&mut self, id: u64, destination_script: &[u8], acquisition_id: u128) -> Result<crate::outbound::TicketFacts, String>;
+    fn outbound_policy(&self, custody_script: &[u8]) -> crate::outbound::BtcPolicy;
@@ async fn owed<I: CustodyIo>(io: &mut I, r: &mut Redemption, reserved: &[Outpoint]) -> Result<String, String> {
     let raw = psbt::finalize(&p, &vec![None; p.tx.inputs.len()], &sigs)?;
     let txid = p.txid();
+    // H4: the bytes against the ticket, independently of the builder. A refusal is loud and final: it is a bug or a forgery.
+    let pre = vec![
+        crate::outbound::Prevout { txid: frog.output.internal_txid(), vout: frog.output.vout, value: frog.value, script: ours.clone() },
+        crate::outbound::Prevout { txid: fee.outpoint.internal_txid(), vout: fee.outpoint.vout, value: fee.value, script: ours.clone() },
+    ];
+    let facts = io.ticket_facts(r.ticket, &dest, r.acquisition_id).await?;
+    let purpose = crate::outbound::Purpose::Delivery { ticket: facts, item: pre[0].clone() };
+    if let Err(e) = crate::outbound::verify_btc(&raw, &pre, &purpose, &io.outbound_policy(&ours)) {
+        r.phase = RPhase::Stuck { why: format!("the outbound check refused the signed delivery: {e:?}") };
+        io.save(r);
+        return Ok(format!("STUCK: outbound check: {e:?}"));
+    }
     // 5. the rule AGAIN, now: signing awaited the subnet, and "immediately before sending" is a number
```

`frogflow::give_back` gets the same block with `Purpose::DepositReturn { deposit, sender_script: dest }`, just before
`d.phase = DPhase::Returning`. The canister's `ticket_facts`:
1. reads `ticket(id)` at the finalized pin (H9);
2. reads `eth_getLogs` for `Redeemed(uint256 indexed ticketId, …)` with `topics[1] = id`, bounded to the finalized block;
3. decodes both destinations with `policy::script_of` for the installed network;
4. sets `names_this_item` from the ticket's acquisition id against `r.acquisition_id`.

Until H9 lands, `outbound_policy` may set `require_finalized = false` and `require_event = false` so H4 can be wired first. The
launch gates require both `true` before value enters custody.

**As wired** (`2562c0e`): both are `true` from the start (`custodynet::outbound_policy`), because H9's ticket read landed with
it. `CustodyIo::ticket_facts(id, acquisition_id)` (the destination script is decoded inside from the ticket, not passed in) reads
`ticket(id)` by `eth_call` at `canister::pin_finalized()` and the `Redeemed` logs by `canister::logs_between` over the last
`REDEEMED_LOOKBACK_BLOCKS` up to that block, every provider agreeing on both; `redeem::ticket_facts_of` decodes them. A ticket not
yet `Resolved`, or without its event, at the finalized block **waits** (nothing is built); a verifier refusal is `Stuck` for a
delivery and `Held` for a return, with nothing saved or sent. The EVM side (`verify_evm`) waits for the NFT delivery path (C10).

**Superseded 2026-09-26 (audit D, H-1; `docs/audit/2026-09-26/fixes-D-canister.md` §2).** With the Phase-1 providers that read
could never succeed: Robinhood's RPC keeps ≈ 6,200 blocks of state and the finalized block is 8,096–12,023 behind the head, and
ordofi refuses `eth_getLogs` over more than 10,000 blocks (the lookback was 8,000,000). `ticket_facts`, `pin_finalized`,
`logs_between` and `REDEEMED_LOOKBACK_BLOCKS` are gone. Now: the ticket's state is read at the fast pin (inside Robinhood's
window), and finality comes from the wrapper's own events, `Redeemed` and `Resolved`, read at or below the finalized block by a
scanner that pages at most 8,000 blocks at a time from a cursor saved in `Custody` (`ticketlog.rs`, `evmread.rs`,
`redeem::facts_of`). `resolved` needs both the state and the final `Resolved` event; `names_this_item` needs both the state and
the event's `acquisitionId`; `event_destination_script` is the final `Redeemed` event's.

---

## H5. Watcher v2 and the custody canary (spec)

Extend `scripts/timelock-watch.mjs`, and add `scripts/custody-canary.mjs` running beside it with the same Telegram channel.
- **Argument diff.** For every `Proposed` upgrade, fetch the proposal's argument (the timelock publishes it). Decode it as `Init`
  with the buyer's `.did`, diff it against the running configuration (a public `config()` query to add, or the last executed
  argument), and post the diff. Any change to `rpc_urls`, `btc_api_urls`, `operator`, `evm_chains`, `custody_caps` or
  `outflow_limits` is marked **CRITICAL** and pages every guardian.
- **Controllers and module hashes.** On every poll, read `/canister/<id>/controllers` and `/canister/<id>/module_hash` for both
  canisters through `read_state` (certified). Alert on any change without a matching `Executed` event.
- **4663 governance.** Watch `ListingRegistry` / `NftRegistry` for every `Propose*`, `setPaused` and `reserveBuyer` event, and the
  keeper's `CapPosted` events (a step above ±20% is an alert until H12 makes it impossible).
- **The canary.** Poll every custody address: the Bitcoin custody, float and buyer addresses through two esplora explorers; the
  EVM address on every configured chain through `eth_getLogs` for ERC-721 `Transfer(from = us)` and native value sent. Every
  outbound transaction must match a record the canister publishes (`redemptions()`, `holder_deposits()`, NFT deliveries, purchase
  lines) **with the same destination**. Anything that does not match (**an unexpected outbound transaction**) pages the guardians
  with the transaction and the words "pause now". The same script reconciles the ledger hourly for the proof-of-reserves page:
  `totalSupply + openTickets × 1e18 == held × 1e18`, and every held item is where the canister says it is.

## H6. The upgrade announcement rule (process)

No `propose_upgrade` without, in the same hour, a public post (repo release plus the project's channel) containing: the commit,
the reproducible module hash (`scripts/reproducible-build.sh`), the decoded argument and its diff (H5), the reason, and the
earliest execution time. A proposal whose post is missing is vetoed by the guardians on sight. That is the guardians' standing
instruction, written in the ceremony (`docs/v5/launch-gates.md`, G2).

## H7. Controller = the timelock only (ceremony)

Already the runbook's intent (`docs/deploy/canister-mainnet.md`, steps 1–7). Made a gate: install, handover and blackhole in one
sitting, witnessed; `dfx canister info` read by two people who hold no role shows the buyer's controllers = `[timelock]` and the
timelock's = `[]`; **no address of the canister receives anything before that reading**. The deployer identity is deleted in
front of the witnesses. `docs/v5/launch-gates.md`, G2, has the full checklist.

## H8. Conflict-out for leaked signed bytes (built at `72531db`)

In `redeem.rs`: when `rule(io, r, signed_unseen = true)` returns Refuse (the bytes were saved, and in the `Sent` → evicted path
they were once in a mempool), do not go straight to `Refused`. Enter a new phase, `Conflicting { leaked_txid, fee_input, raw }`:
1. Build a one-input, one-output transaction spending `fee_input` back to the custody script. Its absolute fee must exceed the
   leaked transaction's fee plus its own vsize × 1 sat/vB (BIP125 rule 4), and stay ≤ `MAX_DELIVERY_FEE_SATS`. Custody's inputs
   already signal replacement (`SEQUENCE = 0xfffffffd`, `src/custody.rs:23`).
2. `outbound::verify_btc(raw, [fee_input prevout], ConflictOut { spend: fee_input }, policy)`, then sign with the custody key,
   save, and send.
3. When the conflict confirms, the redemption becomes `Refused { reason: "…; the leaked delivery was conflicted out in <txid>" }`.
   If Bitcoin instead shows the **leaked delivery** confirmed (it won the race), the record is `LostRace` and the watcher pages,
   exactly as today.

Why the fee input and not the frog: spending the fee input invalidates the leaked delivery just as well, and it never moves the
frog, so no path in this step can send an item anywhere. `frogflow`'s refused-deposit returns get the same step. For EVM
deliveries (when built): a `NonceBurn` at the leaked nonce with a higher fee.

**Left as spec at `2562c0e`.** It is not a hook: it is a new phase in two state machines (`Conflicting`, with its restart
semantics, its fee input reserved, and both races settled), a second custody-key spend built with its own fee rule (the leaked
fee is not in the record; the fee input's value has to be read back from the explorers), and a fake-world test of the eviction.
That is the M the table says, and it was not going to be done cleanly inside the H2–H4 wiring. The entry point is exact:
`redeem::step_inner`, `Signed` → `BtcView::NotYet` → `rule(io, r, true)` returning `Ok(false)`; and `frogflow::step_deposit`'s
`Returning` → not seen path. `outbound::Purpose::ConflictOut` is built and tested for the bytes it must produce.

**Resolved before code, 2026-09-25 (branch `p1-canister`, Phase-1 K1).** The spec above leaves nine things open. Each is decided
here; the tests in `tests/redeem.rs` and `tests/frogs.rs` are written to these words.

1. **When a delivery conflicts out.** Whenever `rule` refuses **saved** bytes (`Signed` → `NotYet` → Refuse), not only after a
   `Sent` → evicted path. `owed` calls `btc_send` right after the save, so any `Signed` bytes may have reached some node even when
   no explorer shows them. The cost of conflicting out bytes that never leaked is one small fee, paid from custody to custody.
   `Refused` stays for refusals before anything was saved (the rule before building and after signing).
2. **One state machine, two records.** The conflict is a pure-plus-async module, `src/conflict.rs`, with a `Conflict` record
   (`leaked_txid`, `raw`, `fee_input`, the leaked item's outpoint, and our conflict transaction once signed). `RPhase` gains
   `Conflicting { reason, conflict }`; `DPhase` gains `ReturnConflicting { reason, conflict }`. Both flows step it through a
   small `ConflictIo` adapter over their own `Io`, so the fee rule, the verifier call and the races are written once.
3. **The fee.** The leaked fee is `fee_input.value − Σ outputs[1..]` of the leaked bytes: output 0 carries the item's whole
   value (H4 checked it before the bytes were saved), and `fee_input.value` is read back from the explorers (all agreeing) with
   its script, which must be custody's. The conflict pays `max(leaked_fee + vsize × 1 sat/vB, vsize × the current rate)`
   (BIP125 rules 3 and 4; its feerate is then above the leaked one's, since it is smaller). It must stay ≤
   `MAX_DELIVERY_FEE_SATS`, and its one output (custody) must be at least the 330-sat dust limit.
4. **One input, or two.** When the fee input alone cannot pay that fee and keep a non-dust output (always the case when the
   leaked delivery had no change output: its whole fee input went to the miner), the conflict adds **one** more custody UTXO,
   chosen exactly as a delivery's fee input is (confirmed, ord shows it carrying nothing, not reserved) and never the leaked item.
   `ConflictOut` already allows custody inputs beside the leaked one. With no such UTXO the conflict **waits** ("the operator
   tops custody up"); it does not give up. Every input of a conflict is reserved from other deliveries, returns and moves until
   the record settles.
5. **The races, checked in this order every step.** (a) The leaked transaction confirmed: it won. A delivery goes to
   `Confirmed { txid: leaked }` and the existing record path decides (`Done` if it gets recorded before the refund, `LostRace`
   if the ticket refunded, with the watcher paging). A return goes to `Returned`. (b) Our conflict confirmed: a delivery becomes
   `Refused { "…; the leaked delivery was conflicted out in <txid>" }`; a return goes back to `Refused` (see 7). (c) The fee input
   spent by a third transaction, **confirmed**: the leaked bytes can never be mined either, so the record settles as in (b),
   naming that transaction. (d) Anything unconfirmed: wait, and send our saved conflict bytes again. A leaked transaction back in a
   mempool does not stop us building or sending the conflict: that is the replacement.
6. **Save, then send; never sign twice.** The signed conflict is saved in the record before it is sent, and a restart sends the
   same bytes. A conflict signed but not saved was never sent, so rebuilding it is safe.
7. **Returns: what "refused" means.** Nothing refuses a return: its bytes send the frog to its sender, which is the intent, so a
   leaked return is not a theft. What H8 prevents for returns is an **evicted return stranded forever** (fee spike, below the
   mempool floor) with its fee input and frog locked. The rule: a `Returning` whose bytes Bitcoin has not shown for
   `RETURN_STALE_SECS` = 72 h (the clock starts at the first `NotYet`, and resets whenever a mempool shows it) conflicts out.
   Once the conflict settles, the deposit goes back to `Refused`, and the next tick builds a fresh return at the current rate
   (subject to the return limits as ever). The old bytes cannot be mined after that, so there are never two live returns.
8. **H1, H2, H3.** A conflict is a custody-key signature, so it **waits while the buyer is paused**, which includes an H3 trip
   (the tick skips custody, and the step checks the pause itself, because a trip can set it mid-tick). It is **not an outflow**:
   everything it spends and pays is custody's, so it neither asks nor records the limiter, and a limiter at its limit does not
   hold it back (a wait there would keep leaked bytes live). The item never moves, so H2's book is untouched; the delivery's
   earlier `ticket:<id>` count stays in the window (conservative). `outbound::verify_btc(…, ConflictOut)` checks every conflict's
   signed bytes before they are saved; a refusal makes the record `Stuck` (a delivery) or `Held` (a return), loudly.
9. **Left open, stated.** A redeemer who re-broadcasts the leaked delivery and hangs a large child off output 0 can pin it
   (BIP125 rule 3 makes our replacement pay for the child). Our fee is capped, so the pin can win, and the result is the
   `LostRace` of 5(a): one item per event, bounded by H3 like any other. We do not bump our own conflict's fee; it is priced at
   the rate of the moment it is built.

## H9. Finality for outbound decisions (spec)

A second pin kind, `pin::finalized`. On 4663: `eth_getBlockByNumber("finalized")` (Nitro reports the L1-finalized L2 block),
rounded down to the bucket, every provider agreeing on its hash. On Ethereum: the `finalized` tag. On Base: `finalized`.
Used for: the ticket read and `Redeemed` log feeding H4; the deposit's EVM side; an NFT arrival before minting
(`docs/v4/nft-wrapper.md` §5.1 already requires it). **Not** used for purchases and quotes, which are judged at the fast pin as
today: a purchase lost to a reorg costs gas, not an item. The latency is ≈ 15–30 minutes on 4663 and Base and ≈ 13 minutes on
Ethereum, against a 7-day delivery deadline.

**Partly wired** (`2562c0e`): `pin::finalized_bucket_body` / `choose_finalized` and `canister::pin_finalized` (the lowest of every
provider's finalized block rounded to the bucket, agreed by hash), `pin::logs_body` and `canister::logs_between` (an `eth_getLogs`
every provider agrees on); used for the ticket read and the `Redeemed` log feeding H4. **Still at the fast pin**: a deposit's EVM
side and an NFT arrival before its claim. Those two are what C6 still asks for.

**Reworked 2026-09-26 (audit D, H-1 and L-7).** A pinned read of STATE at the finalized block cannot be served by the Phase-1 pair
(see H4 above), so on 4663 finality is a block NUMBER used to bound log reads, never a block to read state at:
`evmread::choose_finality` takes the lowest provider's, refuses a provider whose finality is more than 50,000 blocks behind its own
head, or providers more than 8,192 apart, and names the provider it refuses. The events a decision rests on (`Redeemed`,
`Resolved`) are read by `ticketlog::advance` in pages under ordofi's 10,000-block limit, from a saved cursor, and a failing scan is
loud (`TICKET LOGS NOT READ` in the log, `ticket_logs()` for the watcher). An idle custodian reads no logs at all. The fakes in
`tests/fakerpc` carry both providers' measured limits.

## H10. Independent providers, and keys per role (spec)

- **Providers.** Each chain the canister acts on is configured with at least 3 providers run by at least 3 independent operators,
  one of them a node the project runs (for 4663, a Nitro full node with a public HTTPS endpoint: `docs/deploy/canister-mainnet.md`,
  Blocked today 2). Bitcoin: mempool.space, blockstream.info and our own esplora. Record in `deployments/` who operates each
  endpoint and on which infrastructure. dRPC and other aggregators count as the operator they route to, and if that cannot be
  established they do not count.
- **Keys per role.** Today one EVM path (`b"evm"`) is the vault buyer on 4663, every wrapper's minter, and the holder of every NFT
  on every chain (T16). Proposal: `b"evm"` for 4663 operations; `b"mint"` for the wrappers' `custodian`; `b"nft/<chain>"` for
  custody on chain *c*. This contains a signing-path bug to one role. It does **not** help against a subnet compromise (T3), which
  derives every path. It needs `NftWrapper` to take a `holder` separate from the `custodian`, so it is best done before any
  wrapper is deployed. Contracts agent's decision.

## H11. On-chain mint rate limit (spec, Solidity)

In `Wrapper` and `NftWrapper`: an immutable `maxMintsPerDay` (constructor), a ring of the last `maxMintsPerDay` mint times
(timestamp and parent block), and in `wrapDeposit` / `mint` / `mintDeposit`: revert `MintRateLimited` if the oldest of the last
`maxMintsPerDay` mints is younger than 24 h on **either** clock. That is the stricter composition: a sequencer ahead cannot open
room early. `wrap` (creator-driven, from receipted acquisitions) counts too. Suggested: 20 per day per wrapper at stage 0–2,
fixed per deployment. A new wrapper is the only way to change it, which is the point. Tests: the (M+1)th mint in a day reverts;
the sequencer's clock alone cannot open room; the ledger invariants still hold.

## H12. Keeper cap step bound (spec, Solidity)

In `SweepVault.postCap` and `NftVaultBase.postCap`: if a fresh cap exists, the new one must be within ±20% of it
(`MAX_STEP_BPS = 2000`); at most one post per 10 minutes on both clocks; a stale cap may be re-posted at any value, but only
after the owner's 7-day `proposeCapCeiling` has set a ceiling it must stay under. An inflated cap then needs many posts over
hours, all visible to H5. Tests: step bounds, cadence, the ceiling, and that a stale cap does not unlock an arbitrary jump without
the ceiling.

## H13. Cycles (spec)

- **Top-up.** An automated top-up from a cycles-ledger account (or a service such as CycleOps) keeps the buyer above 20 T and the
  timelock above 2 T. Anyone can top up a blackholed canister.
- **Freezing threshold.** 90 days on both canisters, set **before** the handover (after it, only the timelock could set it, and it
  cannot). A gate.
- **Alarms.** At 5 T (warning, existing), 2 T (page every guardian and the operator) and "frozen" (incident).
- **Bounded state.** Finished purchases, redemptions, frog records and deposits older than 90 days are dropped from heap state
  after their final event is noted in the log. The chain holds the history. Add a test that the saved state for 10× stage 3's
  items stays under a fixed size.
- **The `pre_upgrade` drill.** On a replica, fill the state to 10× the stage 3 cap, run an upgrade, and record the instructions
  used against the IC's upgrade limit. Also on a replica: let a canister run out of cycles, confirm the IC uninstalls it, confirm
  the timelock's `upgrade` fails on the empty canister, and rehearse recovery through `propose_controllers`.

## H14. Frontend integrity (spec, web)

Publish every web build's content hash, and an IPFS-pinned copy reachable at a CID announced in the repo. Lock the registrar
and enable DNSSEC. Set a CSP with no third-party script origins and SRI on anything external. Before a redemption is signed, show
the full destination and require its last 6 characters to be typed; for wAPE, default to the connected address and warn on any
other. Show the contract addresses the app will call against `deployments/` in a footer anyone can check. The canary (H5) alerts
when redemptions to never-before-seen destinations exceed 3× their 7-day average.

---

## Verification

On `custody-security` (unwired): `cargo test` with `tests/hardening.rs` at 38 tests (10 custody caps, 10 outflow, 13 outbound, 5
integration, two of them against the real `custody::build`); no Solidity touched; nothing outside `docs/v5/`,
`docs/audit/README.md`, the three modules and `tests/hardening.rs` changed.

On `wire-hardening` (`2562c0e`): `cargo test` in `autobuyer/canister-rs` is green: 272 unit tests (the three modules' 32 now run
with the library's), `tests/hardening.rs` 9 (the integration tests plus the wired `record_target`, stage 0 and the pause ↔
limiter mapping), `tests/redeem.rs` 16 (of which the forged-ticket drain that stops at six and freezes, the delivery to a
destination the finalized ticket does not name refused before saving, the event or item disagreeing, the unresolved ticket
waiting), `tests/frogs.rs` 34 (of which reserve → held, no want without room, a vanished listing freeing its room, an offer over
the value cap, an over-cap deposit returned, a return waiting at the limit, the floor claim counted and limited), `tests/loop.rs`
20, `tests/e2e.rs` 2, `tests/policy_loop.rs` 3. The release wasm (`cargo build --release --target wasm32-unknown-unknown`)
builds: 4,044,088 bytes. No deploy, no chain transaction.

On `p1-canister` (`72531db`, H8; `df2f13c`, Bitcoin mainnet): `cargo test` in `autobuyer/canister-rs` is green at **432** (310 unit, of
which `conflict.rs` 4 and the mainnet checkpoint's header check; `tests/redeem.rs` 25, of which H8's 8: the launch gate's evicted-then-
refused delivery, the leaked delivery winning into `LostRace`, the change-less delivery's second input, two conflicts never
sharing an input, a fee input already spent by a third transaction, a leaked delivery back in a mempool, the pause and the limiter,
a restart and an upgrade mid-conflict; `tests/frogs.rs` 36, of which the two for a stranded return). The timelock's 8 pass.
