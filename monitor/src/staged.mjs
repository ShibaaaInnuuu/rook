// Review round 5 (the operator's option (c), fixes-round5.md §6): the launch plan's staged upgrades, as the launch file names them
// (deployments/launch/nft-pilot.json `stages`: step 1 the install at 48 items and 12 / 24 a day; step 2 the outflow raise, global_items
// 31; step 3 the item caps back to J7's 61). Pure: no network, no clock.
//
// `checkStagedProposal` is golive-check's --proposal read: a pending upgrade's argument is exactly the step it claims to be, from the
// step before it. `stagedStep` is the monitor's: a proposal whose diff from the baseline is exactly one staged step is EXPECTED (a
// WARN, not the SEV1 every loosening is), anything else is judged as ever. Both also hold the proposal's pair to the exit rule
// (canister.rs validate_custody: every item within the timelock's 72 h less a day), which the running code enforces at execute time.
import { diffInit, effectiveCaps, effectiveLimits, exitProblemsOf } from "./argdiff.mjs";

const big = (x) => BigInt(String(x).replace(/_/g, ""));
const LIMIT_KEYS = ["per_collection_items", "global_items", "per_collection_mints", "global_mints", "global_value_wei", "trip_factor"];
/** The fields of an Init that are neither the custody caps nor the outflow limits: a staged step changes none of them. */
const OTHER = ["chain_id", "vault", "key_name", "accept_production_key_name", "bridge_allowlist", "btc_network", "wrapper", "operator",
  "holder_deposits", "wrapper_from_block", "frog_funding", "eth_nft", "rpc_urls", "btc_api_urls", "evm_chains"];

/** A launch file's step `n`, as the caps and limits an Init would name: { caps: {key: {items, value, item}}, global, limits }. */
export function stepNumbers(launch, n) {
  const s = (launch?.stages ?? []).find((x) => Number(x.step) === Number(n));
  if (!s) return null;
  const caps = {};
  for (const c of launch.collections ?? []) {
    const key = `${c.chainId ?? 1}:${String(c.collection).toLowerCase()}`;
    const items = s.custodyItems?.[c.slug] ?? c.custodyItems;
    caps[key] = { items: BigInt(items), value: big(c.custodyWei), item: big(c.custodyWei) };
  }
  // docs/v7/dormant-collections.md §6: a step may NAME its global caps (globalItems, globalWei): with the allowlist's ten collections the
  // global binds (48 / 61 items, 75 ETH) where J7 had it the collections' sum; unnamed, the sum as before
  const binding = s.globalItems !== undefined || s.globalWei !== undefined;
  const global = { items: s.globalItems !== undefined ? BigInt(s.globalItems) : Object.values(caps).reduce((a, c) => a + c.items, 0n),
    value: s.globalWei !== undefined ? big(s.globalWei) : Object.values(caps).reduce((a, c) => a + c.value, 0n) };
  const limits = Object.fromEntries(LIMIT_KEYS.map((k) => [k, big(s.outflow[k])]));
  return { step: Number(s.step), name: s.name, caps, global, limits, binding };
}

/** An Init's (or the canister's) custody caps and limits in the same shape. `caps`: effectiveCaps(); `limits`: effectiveLimits(). */
function pairOf(caps, limits) {
  const c = {};
  for (const x of caps.collections) c[String(x.collection).toLowerCase()] = { items: BigInt(x.max_items), value: big(x.max_value_wei), item: big(x.max_item_wei) };
  return { caps: c, global: { items: BigInt(caps.global_max_items), value: big(caps.global_max_value_wei) },
    limits: Object.fromEntries(LIMIT_KEYS.map((k) => [k, big(limits[k])])) };
}

/** The canister's custody_caps() / custody_outflow() as dfx prints them (JSON, nats with `_`), in the same shape. */
export function runningPair(running) {
  const cp = Array.isArray(running.caps) ? running.caps[0] : running.caps;
  const lp = Array.isArray(running.outflow) ? running.outflow[0] : running.outflow;
  return pairOf(cp, lp);
}

/** The exit rule on a pair (argdiff.mjs `exitProblemsOf`, canister.rs validate_custody): [] when every item can leave in time. */
export const exitProblems = (p) => exitProblemsOf(p.caps, p.global.items, p.limits);

/**
 * Where `p` differs from the step's numbers: the launch collections' own caps, the outflow limits, and (review round 6, A6-L-1) for a
 * PROPOSAL (`exact`) the global caps equal to the sums of its caps (J7: the global item cap is the collections' sum, never binding;
 * the global value the sum of their values; another collection's cap, which `moved` holds unchanged, counts in both). The running pair's global item cap is held from below only (it is compared with the
 * proposal's by `moved`, so a step 2 that keeps it keeps whatever it is). Other collections' caps are compared between pairs (`moved`).
 */
function offStep(p, s, exact = false) {
  const out = [];
  for (const [k, want] of Object.entries(s.caps)) {
    const got = p.caps[k];
    if (!got) { out.push(`${k}: no cap (the step names ${want.items} items)`); continue; }
    for (const f of ["items", "value", "item"]) if (got[f] !== want[f]) out.push(`${k} ${f}: ${got[f]} (the step: ${want[f]})`);
  }
  if (s.binding) {
    // docs/v7/dormant-collections.md §6: the step names a binding global: the pair holds exactly it
    if (p.global.items !== s.global.items) out.push(`the global item cap ${p.global.items} is not the step's ${s.global.items} (a binding global: docs/v7/dormant-collections.md §6)`);
    if (p.global.value !== s.global.value) out.push(`the global value cap ${p.global.value} is not the step's ${s.global.value}`);
  } else if (exact) {
    const sum = (f) => Object.values(p.caps).reduce((a, c) => a + c[f], 0n);
    if (p.global.items !== sum("items")) out.push(`the global item cap ${p.global.items} is not the caps' sum ${sum("items")} (J7: the global is the sum, no total binds)`);
    if (p.global.value !== sum("value")) out.push(`the global value cap ${p.global.value} is not the caps' sum ${sum("value")}`);
  } else if (p.global.items < s.global.items) out.push(`the global item cap ${p.global.items} is under the collections' sum ${s.global.items} (J7: no total binds)`);
  for (const k of LIMIT_KEYS) if (p.limits[k] !== s.limits[k]) out.push(`outflow ${k}: ${p.limits[k]} (the step: ${s.limits[k]})`);
  return out;
}

/**
 * Review round 6 (A6-I-4): the running code's ×4 rule (custodycap::validate_change, outflow's check_change): no cap or limit raised
 * more than ×4 in one upgrade. The raises past it, as text.
 */
function overFour(a, b) {
  const out = [];
  const x4 = (what, x, y) => { if (x > 0n && y > x * 4n) out.push(`${what} ${x} → ${y} (more than ×4 at once: the running code refuses it)`); };
  for (const [k, c] of Object.entries(b.caps)) {
    const o = a.caps[k];
    if (o) for (const f of ["items", "value", "item"]) x4(`${k} ${f}`, o[f], c[f]);
  }
  x4("the global item cap", a.global.items, b.global.items);
  x4("the global value cap", a.global.value, b.global.value);
  for (const k of LIMIT_KEYS) if (k !== "trip_factor") x4(`outflow ${k}`, a.limits[k], b.limits[k]);
  return out;
}

/** What moved between two pairs: { caps: [keys], global: bool, limits: [keys] }. */
function moved(a, b) {
  const keys = new Set([...Object.keys(a.caps), ...Object.keys(b.caps)]);
  const caps = [...keys].filter((k) => JSON.stringify(a.caps[k] ?? null, (_, v) => (typeof v === "bigint" ? String(v) : v))
    !== JSON.stringify(b.caps[k] ?? null, (_, v) => (typeof v === "bigint" ? String(v) : v)));
  const global = a.global.items !== b.global.items || a.global.value !== b.global.value;
  const limits = LIMIT_KEYS.filter((k) => a.limits[k] !== b.limits[k]);
  return { caps, global, limits };
}

/**
 * Is the move from `prev` to `next` exactly step `n`'s own kind of change? A reason when not. `launchKeys`: the launch collections'
 * cap keys. Review round 6 (A6-L-1): step 3 moves the launch collections' caps and the global caps (to their sums) and nothing else:
 * another collection's cap (a Bitcoin wrapper's, a new key, a removed one) moving is not step 3.
 */
function kindProblem(n, m, launchKeys = []) {
  if (n === 2) {
    if (m.caps.length || m.global) return `step 2 changes the outflow alone, but the custody caps move too (${m.caps.join(", ") || "the global cap"})`;
    if (m.limits.join(",") !== "global_items") return `step 2 changes global_items alone, but ${m.limits.join(", ") || "nothing"} moves`;
  } else if (n === 3) {
    if (m.limits.length) return `step 3 changes the item caps alone, but the outflow moves too (${m.limits.join(", ")})`;
    if (!m.caps.length) return "step 3 changes no item cap";
    const others = m.caps.filter((k) => !launchKeys.includes(k));
    if (others.length) return `step 3 changes the launch collections' item caps alone, but other caps move too (${others.join(", ")})`;
  } else return `step ${n} is not a staged upgrade (steps 2 and 3 are)`;
  return null;
}

/** Review round 6 (A6-L-1): the diff lines (argdiff.mjs diffInit, by their `tag`) that step `n` itself names: only these are EXPECTED. */
export function stepTags(launch, n) {
  const s = stepNumbers(launch, n);
  if (!s) return [];
  if (Number(n) === 2) return ["limit:global_items"];
  if (Number(n) === 3) return [...Object.keys(s.caps).map((k) => `cap:${k}:max_items`), "global:global_max_items"];
  return [];
}

/**
 * golive-check --proposal: `next` and `base` are decodeInit() results (`base` may be null), `running` the canister's pair as dfx printed
 * it (may be null; one of the two is needed), `launch` the launch file, `step` 2 or 3. { ok, lines }.
 */
export function checkStagedProposal({ next, base, launch, step, running, requireBaseline = false }) {
  const lines = [];
  let ok = true;
  const say = (good, text) => { lines.push(`${good ? "ok  " : "FAIL"} ${text}`); ok &&= good; };
  const s = stepNumbers(launch, step), prev = stepNumbers(launch, step - 1);
  if (!s || !prev) { say(false, `the launch file names no step ${step} after a step ${step - 1}`); return { ok, lines }; }
  if (!next?.ok) { say(false, `the proposal's argument: ${next?.error ?? "not decoded"}`); return { ok, lines }; }
  if (base && !base.ok) { say(false, `the baseline argument: ${base.error}`); return { ok, lines }; }
  const p = pairOf(effectiveCaps(next.init), effectiveLimits(next.init));
  const off = offStep(p, s, true);
  say(!off.length, `the proposal's custody caps and outflow limits are step ${step} (${s.name}) of the launch file${off.length ? `: ${off.join("; ")}` : ""}`);
  const r = running ? runningPair(running) : pairOf(effectiveCaps(base.init), effectiveLimits(base.init));
  const offPrev = offStep(r, prev);
  say(!offPrev.length, `the running configuration (${running ? "the canister's custody_caps() and custody_outflow()" : "the baseline argument"}) is step ${step - 1} (${prev.name})${offPrev.length ? `: ${offPrev.join("; ")}` : ""}`);
  const kp = kindProblem(step, moved(r, p), Object.keys(s.caps));
  say(!kp, kp ?? `the change is step ${step}'s alone (${step === 2 ? "outflow_limits.global_items" : "the launch collections' item counts, the global item cap their sum"}), nothing else in the caps or limits`);
  const x4 = overFour(r, p);
  say(!x4.length, x4.length ? `the proposal raises past the running code's ×4 rule: ${x4.join("; ")}` : "no cap or limit is raised more than ×4 at once (the running code's rule)");
  if (!base && requireBaseline) say(false, "no --baseline: under --phase1 the proposal must be diffed against the argument it replaces (the last executed upgrade's, or the install's), so every other Init field is compared (review round 6, A6-I-2)");
  if (base) {
    const { unchanged } = diffInit(base.init, next.init);
    const others = OTHER.filter((f) => !unchanged.includes(f));
    say(!others.length, others.length ? `the proposal changes more than the step: ${others.join(", ")}` : "every other field of the Init is the baseline's (identity, operator, providers, chains, eth_nft, deposits)");
  } else lines.push("note no --baseline: the Init's other fields (identity, operator, providers, chains, eth_nft) were not compared; the monitor's argument diff shows them");
  const ex = exitProblems(p);
  say(!ex.length, ex.length ? `the proposal does not fit the exit rule (the running code refuses it at execute time): ${ex.join("; ")}` : "the proposal's pair fits the exit rule: every item can leave within the timelock's 72 h less a day (A4-L-1)");
  return { ok, lines };
}

/**
 * The monitor's: is the diff from `baseInit` to `nextInit` exactly one staged step of `launch`? The step number, or null. (Only the
 * fields a step may change moved, and the base and next pairs are that step's and the one before it.)
 */
export function stagedStep(launch, baseInit, nextInit) {
  if (!launch?.stages?.length) return null;
  const { unchanged } = diffInit(baseInit, nextInit);
  if (OTHER.some((f) => !unchanged.includes(f))) return null;
  const a = pairOf(effectiveCaps(baseInit), effectiveLimits(baseInit)), b = pairOf(effectiveCaps(nextInit), effectiveLimits(nextInit));
  for (const n of [2, 3]) {
    const s = stepNumbers(launch, n), prev = stepNumbers(launch, n - 1);
    if (!s || !prev) continue;
    if (!offStep(b, s, true).length && !offStep(a, prev).length && !kindProblem(n, moved(a, b), Object.keys(s.caps)) && !overFour(a, b).length
      && !exitProblems(b).length) return n;
  }
  return null;
}
