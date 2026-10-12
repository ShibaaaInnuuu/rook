// Watcher v2 (docs/v6/k6-watcher-canary.md §2, §3): the timelock's events in v1's words, every proposal with its decoded
// argument diffed against the running one and its module hash against the published builds, the certified controllers and
// module hashes, the live caps against the baseline, the tripwire, and cycles.
import { describe, surprises } from "../../scripts/timelock-watch.mjs";
import { decodeInit } from "./candid.mjs";
import { buildOf, effectiveCaps, effectiveLimits, proposalAlert } from "./argdiff.mjs";
import { stagedStep, stepTags } from "./staged.mjs";

const opt = (x) => (Array.isArray(x) ? (x.length ? x[0] : null) : x ?? null);
const big = (x) => BigInt(String(x).replace(/_/g, ""));
const sameSet = (a, b) => JSON.stringify([...(a ?? [])].sort()) === JSON.stringify([...(b ?? [])].sort());
const jsonBig = (v) => JSON.stringify(v, (_, x) => (typeof x === "bigint" ? x.toString() : x));
export const PAGE_CYCLES = 2_000_000_000_000n;   // G8: 2 T pages everyone

/** The running configuration's argument: the last executed upgrade in the history, else the recorded install argument. Pure. */
export function baselineOf(history, installArgHex) {
  const proposed = new Map();
  let last = null;
  for (const ev of history) {
    const [k, e] = Object.entries(ev)[0];
    if (k === "Proposed") proposed.set(String(e.id), e.view);
    if (k === "Executed") {
      const v = proposed.get(String(e.id));
      if (v?.kind === "upgrade" && opt(v.arg_hex)) last = { hex: opt(v.arg_hex), source: `the argument of upgrade ${e.id}, executed at ${new Date(Number(big(e.at_ns) / 1_000_000n)).toISOString()}` };
    }
  }
  const pick = last ?? (installArgHex ? { hex: installArgHex, source: "the install argument recorded at the ceremony" } : null);
  if (!pick) return { baseline: null, error: null };
  const d = decodeInit(pick.hex);
  if (!d.init) return { baseline: null, error: `${pick.source} does not decode: ${d.error}` };
  return { baseline: { init: d.init, source: pick.source }, error: d.ok ? null : d.error };
}

/** The canister's live caps and limits against the baseline's effective ones. Pure: [] when they agree. */
export function baselineMismatch(baseline, capsReply, outflowReply) {
  const out = [];
  const norm = (c) => { const e = effectiveCaps({ custody_caps: [c] }); e.collections.sort((a, b) => a.collection.localeCompare(b.collection)); return jsonBig(e); };
  const want = effectiveCaps(baseline.init);
  want.collections.sort((a, b) => a.collection.localeCompare(b.collection));
  if (jsonBig(want) !== norm(capsReply[0])) out.push(`custody_caps(): the canister has ${norm(capsReply[0])}, the baseline says ${jsonBig(want)}`);
  const wl = jsonBig(effectiveLimits(baseline.init)), gl = jsonBig(effectiveLimits({ outflow_limits: [outflowReply[0]] }));
  if (wl !== gl) out.push(`custody_outflow(): the canister has ${gl}, the baseline says ${wl}`);
  return out;
}

/**
 * Certified controllers and module hashes. Pure. `now`/`prev`: { [id]: { controllers, module_hash } }. `executed`: whether an
 * `Executed` event is new in this poll. `pending`: the timelock's pending proposals (an upgrade that is running right now has
 * changed the module before its `Executed` is recorded: that is given one more poll).
 */
export function certifiedAlerts({ buyer, timelock, now, prev, builds, executed, pending, nowNs }) {
  const alerts = [], hold = [];
  const b = now[buyer], t = now[timelock];
  if (!sameSet(b.controllers, [timelock])) alerts.push({ kind: "controllers", sev: "SEV1", persistent: true,
    key: `controllers:${buyer}:${[...b.controllers].sort().join(",")}`,
    text: `THE BUYER ${buyer} IS NOT CONTROLLED BY THE TIMELOCK ALONE (certified): controllers are ${b.controllers.join(", ") || "(none)"}, expected exactly ${timelock}. Whoever controls it can replace its code with no delay. Pause now.` });
  if (t.controllers.length) alerts.push({ kind: "controllers", sev: "SEV1", persistent: true,
    key: `controllers:${timelock}:${[...t.controllers].sort().join(",")}`,
    text: `THE TIMELOCK ${timelock} IS NOT BLACKHOLED (certified): controllers are ${t.controllers.join(", ")}. Whoever that is can replace the timelock, and with it every delay.` });
  const bb = buildOf(builds, "buyer", b.module_hash), tb = buildOf(builds, "timelock", t.module_hash);
  const rehearsal = buildOf(builds, "timelock_rehearsal", t.module_hash);
  if (!bb) alerts.push({ kind: "controllers", sev: "SEV1", persistent: true, key: `module:${buyer}:${b.module_hash}`,
    text: `the buyer ${buyer} runs module ${b.module_hash ?? "(none: empty or uninstalled)"} (certified), which is NOT a published reproducible build. Verify: autobuyer/scripts/reproducible-build.sh verify ${buyer} buyer ic` });
  if (!tb) alerts.push({ kind: "controllers", sev: "SEV1", persistent: true, key: `module:${timelock}:${t.module_hash}`,
    text: rehearsal
      ? `the timelock ${timelock} runs the REHEARSAL build (${t.module_hash}, a 10-second floor): it must never guard the real buyer.`
      : `the timelock ${timelock} runs module ${t.module_hash ?? "(none)"} (certified), which is NOT the published timelock build.` });

  for (const [id, label] of [[buyer, "buyer"], [timelock, "timelock"]]) {
    const p = prev?.[id], n = now[id];
    if (!p) continue;
    const moved = [];
    if (p.module_hash !== n.module_hash) moved.push(`module hash ${p.module_hash} → ${n.module_hash}`);
    if (!sameSet(p.controllers, n.controllers)) moved.push(`controllers ${p.controllers.join(", ") || "(none)"} → ${n.controllers.join(", ") || "(none)"}`);
    if (!moved.length) continue;
    if (executed) {
      const known = label === "buyer" ? bb : tb;
      alerts.push({ kind: "governance", sev: "INFO", key: `changed:${id}:${n.module_hash}:${n.controllers.join(",")}`,
        text: `the ${label} ${id} changed after an executed proposal (certified): ${moved.join("; ")}${label === "buyer" && p.module_hash !== n.module_hash ? `. The new module is ${known ? `the reproducible build of ${known.commit}` : "NOT a published build"}` : ""}.` });
      continue;
    }
    const running = label === "buyer" && (pending ?? []).some((v) => big(v.executable_ns) <= big(nowNs ?? 0n) &&
      ((v.kind === "upgrade" && opt(v.wasm_hash) === n.module_hash) || (v.kind === "controllers" && sameSet(opt(v.controllers), n.controllers))));
    if (running) { hold.push(id); continue; }
    alerts.push({ kind: "controllers", sev: "SEV1", persistent: true, key: `unexecuted:${id}:${n.module_hash}:${[...n.controllers].sort().join(",")}`,
      text: `THE ${label.toUpperCase()} ${id} CHANGED WITHOUT AN EXECUTED PROPOSAL (certified): ${moved.join("; ")}. Nothing the timelock did explains it (G10 SEV1): pause now, and read the timelock's history().` });
  }
  return { alerts, hold };
}

/** Cycles, at the canister's own threshold and at G8's 2 T for the buyer. Pure; one alert per crossing, each way. */
export function cyclesCheck(prev, now) {
  const alerts = [], next = {};
  const t = (x) => `${(Number(big(x) / 1_000_000_000n) / 1000).toFixed(2)}T`;
  for (const [label, c] of Object.entries(now)) {
    const levels = { low: big(c.balance) < big(c.low_threshold), page: label.startsWith("the buyer") && big(c.balance) < PAGE_CYCLES };
    const p = prev?.[label] ?? { low: false, page: false, n: 0 };
    let n = p.n ?? 0;
    for (const lvl of ["low", "page"]) {
      if (levels[lvl] === !!p[lvl]) continue;
      n++;
      const what = lvl === "low" ? `its own ${t(c.low_threshold)} threshold` : "2.00T (G8: page everyone)";
      alerts.push({ kind: "cycles", sev: "SEV2", key: `cycles:${label}:${lvl}:${levels[lvl]}:${n}`,
        text: levels[lvl]
          ? `🔋 ${label} is LOW on cycles: ${t(c.balance)} left, below ${what}. Anyone can top it up: dfx cycles top-up <id> <amount> --network ic. A frozen buyer cannot deliver (T13).`
          : `🔋 ${label} is above ${what} again: ${t(c.balance)}` });
    }
    next[label] = { ...levels, n };
  }
  return { alerts, next };
}

/**
 * Go-live cycles plan (docs/v7/golive-runbook.md §6): the buyer's BURN RATE, which the level alerts above cannot see coming. Worst case
 * measured by the review (fixes-round5.md §3: the sale scan's bounds): ≈ 10.5 T a day a collection, ≈ 52 T for five, on a 13-node
 * subnet (×2.6 on a 34-node one), which drains a 5 T threshold in hours. Defaults, each overridable in the configuration's `cycles`:
 */
export const BURN_DEFAULTS = Object.freeze({
  window_h: 24,            // the trailing window the rate is measured over (at least BURN_MIN_SPAN_S of samples before any page)
  min_days: 7,             // SEV2 when the balance lasts fewer days than this at the measured rate
  min_days_sev1: 2,        // SEV1 (persistent) under this many days
  burn_warn_t_per_day: 10, // SEV2 when the rate itself is above this (docs/v7/dormant-collections.md §7, §11: ≈ 0.1–0.3 T idle, ≈ 1.5–2.5 T per ACTIVE collection; the launch balance keeps one awake, ≈ 2.8 T; raise it as top-ups raise the limit, 13 nodes): an attack or a loop
  min_balance_t: 12,       // SEV2 under this balance whatever the rate: the dormancy's reserve (docs/v7/dormant-collections.md §11: the canister's 8 T scan floor, under which it stops its own sale scan, plus one awake collection's 3 T exit reserve)
});
export const BURN_SAMPLE_S = 600;       // one kept sample every 10 min (the watcher polls every minute)
export const BURN_MIN_SPAN_S = 6 * 3600;

/**
 * Pure. `prev`: { [label]: { s: [[seconds, balance string], ...], lvl: {..} } }; `now`: { [label]: { balance } } (the same labels as
 * cyclesCheck); `nowS`: seconds. The rate is what the balance LOST over the window (a top-up is a rise, never counted against it) per day;
 * the days left are the balance over that rate. One alert per crossing, each way; SEV1 persistent while it lasts. The rate and the floor
 * page for the buyer only (as G8's 2 T); the days left for both canisters.
 */
export function burnCheck(prev = {}, now, nowS, cfg = {}) {
  const c = { ...BURN_DEFAULTS, ...(cfg ?? {}) };
  const T = 1_000_000_000_000n, t = (x) => `${(Number(big(x) / 1_000_000_000n) / 1000).toFixed(2)}T`;
  const alerts = [], next = {}, report = {};
  for (const [label, cy] of Object.entries(now)) {
    const p = prev?.[label] ?? {}, buyer = label.startsWith("the buyer");
    let s = (p.s ?? []).filter(([at]) => at >= nowS - c.window_h * 3600 && at <= nowS);
    const bal = big(cy.balance);
    if (!s.length || nowS - s[s.length - 1][0] >= BURN_SAMPLE_S) s = [...s, [nowS, bal.toString()]];
    const span = s.length > 1 ? s[s.length - 1][0] - s[0][0] : 0;
    let lost = 0n;
    for (let i = 1; i < s.length; i++) { const d = big(s[i - 1][1]) - big(s[i][1]); if (d > 0n) lost += d; }
    const perDay = span >= BURN_MIN_SPAN_S ? (lost * 86_400n) / BigInt(span) : null;
    const days = perDay && perDay > 0n ? Number((bal * 1000n) / perDay) / 1000 : null;
    const lvl = {
      sev1: days !== null && days < c.min_days_sev1,
      days: days !== null && days < c.min_days,
      // the rate and the floor are the buyer's numbers (the timelock is topped up to ≈ 2 T and burns almost nothing); the days, both's
      rate: buyer && perDay !== null && perDay > BigInt(Math.round(c.burn_warn_t_per_day * 1000)) * (T / 1000n),
      floor: buyer && bal < BigInt(Math.round(c.min_balance_t * 1000)) * (T / 1000n),
    };
    const was = p.lvl ?? {};
    let n = p.n ?? 0;
    const rateText = perDay !== null ? `${t(perDay)} a day over the last ${(span / 3600).toFixed(1)} h` : "not measured yet";
    const daysText = days !== null ? `${days.toFixed(1)} days` : "no measured burn";
    const top = `Top it up now (anyone can: dfx cycles top-up <id> <amount> --network ic, or dfx canister deposit-cycles); the policy is docs/v7/golive-runbook.md §6.`;
    const say = {
      sev1: [`🔋 ${label} has about ${daysText} of cycles left at its burn rate (${t(bal)}, burning ${rateText}): UNDER ${c.min_days_sev1} DAYS. Under 8 T the canister stops its own sale scan; frozen, it cannot deliver. ${top}`, "SEV1", true],
      days: [`🔋 ${label} has about ${daysText} of cycles left at its burn rate (${t(bal)}, burning ${rateText}): under ${c.min_days} days. ${top}`, "SEV2", false],
      rate: [`🔥 ${label} is burning ${rateText}, above ${c.burn_warn_t_per_day} T a day (the dormant model's normal: ≈ 0.1–0.3 T idle and ≈ 1.5–2.5 T per ACTIVE collection, as many awake as the balance keeps (eth_capacity), 13 nodes; docs/v7/dormant-collections.md §7, §11): an evidence or scan flood (the scan's bounded worst case is ≈ 10.5 T a day per awake collection), a stuck retry, a wake-request flood, or more collections awake than planned (eth_nft_collections()). Balance ${t(bal)}, ≈ ${daysText}. Look at the canister's tick notes and the runner's hand-ins.`, "SEV2", false],
      floor: [`🔋 ${label} is under ${c.min_balance_t} T of cycles (${t(bal)}; burning ${rateText}). Under 8 T the canister stops its own sale scan. ${top}`, "SEV2", false],
    };
    for (const k of ["sev1", "days", "rate", "floor"]) {
      if (lvl[k] === !!was[k]) { if (lvl[k] && say[k][2]) alerts.push({ kind: "cycles", sev: say[k][1], persistent: true, key: `cycles-burn:${label}:${k}:${n}`, text: say[k][0] }); continue; }
      n++;
      alerts.push(lvl[k]
        ? { kind: "cycles", sev: say[k][1], persistent: say[k][2], key: `cycles-burn:${label}:${k}:${n}`, text: say[k][0] }
        : { kind: "cycles", sev: "INFO", key: `cycles-burn:${label}:${k}:${n}`, text: `🔋 ${label}: back from "${k}" (${t(bal)}, burning ${rateText}, ≈ ${daysText}).` });
    }
    next[label] = { s, lvl, n };
    report[label] = { balance: bal.toString(), per_day: perDay === null ? null : perDay.toString(), days_left: days, span_h: +(span / 3600).toFixed(2) };
  }
  return { alerts, next, report };
}

/**
 * The watcher's poll. `ic`: { status, history, cycles(id), certified(id), custodyCaps(id), custodyOutflow(id) }, all
 * returning plain values. Returns { alerts, next, status } and changes nothing.
 */
export async function checkWatcher({ cfg, prev = {}, ic, now = Date.now() }) {
  const tl = cfg.timelock;
  const alerts = [];
  const [status, history] = await Promise.all([ic.status(), ic.history()]);
  status.timelock = tl;
  const buyer = cfg.buyer ?? status.target;

  const wrong = surprises(status, cfg.expect ?? {}, cfg.network ?? "ic");
  if (cfg.buyer && status.target !== cfg.buyer) wrong.push(`target is ${status.target}, but the monitor watches buyer ${cfg.buyer}`);
  if (wrong.length) alerts.push({ kind: "config", sev: "SEV1", persistent: true, key: `config:${tl}:${wrong.join("|")}`,
    text: `the buyer timelock ${tl} is not configured as expected:\n- ${wrong.join("\n- ")}` });

  const { baseline, error } = baselineOf(history, cfg.install_arg_hex);
  if (error) alerts.push({ kind: "config", sev: "SEV1", persistent: true, key: `baseline-error:${error}`, text: `the baseline argument is not usable: ${error}` });
  if (!baseline && !error) alerts.push({ kind: "config", sev: "SEV2", persistent: true, key: "baseline:none",
    text: "no baseline for the buyer's running configuration: set install_arg_hex (the pre-handover install argument, from the ceremony log). Until then no upgrade argument can be diffed." });

  // certified reads after the history, so an upgrade executed between the two is seen as "running" (one poll's grace)
  const cert = { [buyer]: await ic.certified(buyer), [tl]: await ic.certified(tl) };

  const seen = prev.seen ?? null;
  const fresh = seen === null || seen > history.length ? [] : history.slice(seen);
  const executed = fresh.some((ev) => Object.keys(ev)[0] === "Executed");
  const ctx = (v) => {
    const decoded = v.kind === "upgrade" ? decodeInit(opt(v.arg_hex)) : null;
    // review round 5 (option (c)): a proposal that is exactly a staged step of the launch plan (cfg.launch: the launch file) is expected
    const staged = decoded?.ok && baseline?.init && cfg.launch ? stagedStep(cfg.launch, baseline.init, decoded.init) : null;
    // review round 6 (A6-L-1): the lines the step itself names, the only ones downgraded
    return { timelock: tl, builds: cfg.builds, baseline, roles: status.roles, controllers: cert[buyer].controllers, decoded, staged,
      stagedTags: staged ? stepTags(cfg.launch, staged) : null };
  };
  const persist = (a) => ({ ...a, persistent: a.sev === "SEV1" });

  if (seen === null) alerts.push({ kind: "governance", sev: "INFO", key: `watching:${tl}:${history.length}`,
    text: `👀 watching the buyer timelock ${tl} (target ${status.target}; delay ${(Number(big(status.delay_secs)) / 3600).toFixed(1)} h; ${history.length} past events; ${status.pending.length} pending)` });
  // canister review2 N-6: a proposal being executed (the buyer stopped for it); one cancelled meanwhile is not installed
  for (const [v, why] of opt(status.running) ?? []) alerts.push({ kind: "governance", sev: "SEV2", key: `running:${tl}:${v.id}:${opt(why) ?? "-"}`,
    text: `the buyer timelock ${tl} is EXECUTING proposal ${v.id} (${v.kind}) now: the buyer is stopped for it${opt(why) ? `; it was ${opt(why)} meanwhile, so nothing is installed and the buyer is started again` : ""}. A second execute is refused while it runs.` });
  // every pending proposal, every poll (a SEV1 one repeats every 6 h until it is gone)
  for (const v of status.pending ?? []) alerts.push(persist(proposalAlert(v, ctx(v))));
  fresh.forEach((ev, i) => {
    const [k, e] = Object.entries(ev)[0];
    if (k === "Proposed") { alerts.push(proposalAlert(e.view, ctx(e.view))); return; }   // proposed and gone within one poll
    // audit D, L-2: NotStarted means the upgrade installed and the buyer did NOT start again (nothing delivers): SEV1
    const sev = k === "NotStarted" || (k === "Started" && !e.ok) ? "SEV1"
      : (k === "Paused" && (e.paused || !e.ok)) || k === "Failed" || k === "NotStopped" || k === "NotUploaded" || k === "Started" ? "SEV2" : "INFO";
    alerts.push({ kind: "governance", sev, persistent: sev === "SEV1", key: `event:${tl}:${seen + i}`, text: describe(ev, status) });
  });

  const c = certifiedAlerts({ buyer, timelock: tl, now: cert, prev: prev.certified, builds: cfg.builds, executed,
    pending: status.pending, nowNs: status.now_ns });
  alerts.push(...c.alerts);
  const certified = { ...cert };
  for (const id of c.hold) certified[id] = prev.certified[id];   // compare again next poll

  const [capsNow, outflowNow] = await Promise.all([ic.custodyCaps(buyer), ic.custodyOutflow(buyer)]);
  if (baseline) {
    const m = baselineMismatch(baseline, capsNow, outflowNow);
    if (m.length) alerts.push({ kind: "config", sev: "SEV1", persistent: true, key: `baseline-mismatch:${m.join("|")}`,
      text: [`the monitor's picture of the buyer's configuration (${baseline.source}) does NOT match the canister's own answer:`, ...m.map((x) => `- ${x}`),
        "A diff against a wrong baseline proves nothing. Find out why: an upgrade this monitor did not see, or the wrong install argument configured."].join("\n") });
  }
  const freeze = outflowNow[1]?.freeze ?? {};
  const acked = opt(outflowNow[1]?.acked) ?? [];
  if (freeze.Frozen) alerts.push({ kind: "config", sev: "SEV2", persistent: true, key: `tripwire:${freeze.Frozen.since}`,
    text: `custody is FROZEN by the tripwire since ${new Date(Number(big(freeze.Frozen.since) / 1_000_000n)).toISOString()}: ${JSON.stringify(freeze.Frozen.cause)}. G10 SEV2: assess within an hour; resuming needs two guardians.${acked.length ? ` ${acked.length} key(s) acknowledged by the last resume still wait on the limit without counting toward a trip (${acked.slice(0, 5).map((x) => `${x.key} ${Object.keys(x.flow)[0]}`).join(", ")}): a NEW key tripped this freeze.` : ""}` });

  const cyc = {};
  for (const [label, id] of [[`the timelock ${tl}`, tl], [`the buyer ${buyer}`, buyer]]) cyc[label] = await ic.cycles(id);
  const cy = cyclesCheck(prev.cycles, cyc);
  alerts.push(...cy.alerts);
  // go-live (docs/v7/golive-runbook.md §6): the burn rate and the days it leaves
  const burn = burnCheck(prev.burn, cyc, Math.floor(now / 1000), cfg.cycles);
  alerts.push(...burn.alerts);

  return { alerts, next: { seen: history.length, certified, cycles: cy.next, burn: burn.next, burn_report: burn.report, buyer }, status };
}
