#!/usr/bin/env node
// docs/deploy/canister-mainnet.md, "Watching the delay". A public delay protects only the people who read it (docs/v2/timelock-
// controller.md, "Does not protect against: nobody watching"). This reads the buyer's timelock and posts every new event —
// Proposed, Cancelled (withdrawn), VetoVote, Vetoed, Executed, Failed, Paused, ResumeVote, RolesChanged, NotStopped, NotStarted, NotUploaded, Started — to a Telegram chat, so a
// proposal is seen on the day it is made, with its module hash, its argument's hash and the moment it can run. It also reads both
// canisters' public `cycles()` and alerts once when either falls below the threshold the canister itself publishes, and once when
// it recovers (docs/v2/trust-points.md §2): after the handover nobody can read them any other way.
//
// It only READS: `status` and `history` are public queries, asked as the anonymous identity. It holds no key and can change
// nothing. The Telegram bot token and chat id are read from the environment (TELEGRAM_BOT_TOKEN, TELEGRAM_CHAT_ID) and the
// token is never printed, logged or written to the state file.
//
//   node scripts/timelock-watch.mjs --timelock <id> [--network ic|local] [--once] [--dry-run] [--every 60] [--certified]
//        [--state ~/.etch-timelock-watch.json] [--expect-target <id>] [--expect-proposers <p,p>] [--expect-guardians <p,p,p>]
//        [--expect-veto <k>] [--expect-delay <secs>]
//
//   --dry-run     print each message instead of posting it (the default when no token is set)
//   --certified   ask as update calls, which go through consensus and cannot be forged by a boundary node; each costs the
//                 timelock a few million cycles. Without it the answers are uncertified queries — fine to alert on, and every
//                 alert tells the reader to check the hash against their own reproducible build anyway.
import { execFile } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const CANDID = join(HERE, "..", "autobuyer", "timelock-rs", "etch_timelock.did");
const BUYER_CANDID = join(HERE, "..", "autobuyer", "canister-rs", "etch_autobuyer.did");
const DFX = process.env.DFX ?? join(homedir(), "Library/Application Support/org.dfinity.dfx/bin/dfx");

export function parseArgs(argv) {
  const o = { network: "ic", once: false, dryRun: false, every: 60, certified: false,
    state: join(homedir(), ".etch-timelock-watch.json"), expect: {} };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i], v = () => { if (i + 1 >= argv.length) throw new Error(`${a} needs a value`); return argv[++i]; };
    if (a === "--timelock") o.timelock = v();
    else if (a === "--network") o.network = v();
    else if (a === "--once") o.once = true;
    else if (a === "--dry-run") o.dryRun = true;
    else if (a === "--certified") o.certified = true;
    else if (a === "--every") o.every = Number(v());
    else if (a === "--state") o.state = v();
    else if (a === "--expect-target") o.expect.target = v();
    else if (a === "--expect-proposers") o.expect.proposers = v().split(",").map((x) => x.trim()).filter(Boolean);
    else if (a === "--expect-guardians") o.expect.guardians = v().split(",").map((x) => x.trim()).filter(Boolean);
    else if (a === "--expect-veto") o.expect.veto = v();
    else if (a === "--expect-delay") o.expect.delay = v();
    else throw new Error(`unknown argument ${a}`);
  }
  if (!o.timelock || !/^[a-z0-9]{5}(-[a-z0-9]{5}){3,}-[a-z0-9]{3}$/.test(o.timelock)) throw new Error("--timelock <canister id> is required");
  if (!["ic", "local"].includes(o.network)) throw new Error("--network is ic or local");
  if (!(o.every >= 10)) throw new Error("--every is at least 10 seconds");
  return o;
}

/** One call to the timelock, as the anonymous identity. `dfx` prints the reply as JSON on stdout; warnings go to stderr. */
function ask(o, method, canister = o.timelock, candid = CANDID) {
  const args = ["canister", "call", "--network", o.network, "--identity", "anonymous", "--candid", candid, "--output", "json",
    o.certified ? "--update" : "--query", canister, method];
  return new Promise((ok, fail) => execFile(DFX, args, { timeout: 120_000, env: { ...process.env, DFX_WARNING: "-mainnet_plaintext_identity" } },
    (err, stdout) => {
      if (err) return fail(new Error(`${method}: ${String(err.message).split("\n")[0]}`));
      try { ok(JSON.parse(stdout)); } catch { fail(new Error(`${method}: the reply is not JSON`)); }
    }));
}

const opt = (x) => (Array.isArray(x) ? (x.length ? x[0] : null) : x ?? null);
const when = (ns) => new Date(Number(BigInt(ns) / 1_000_000n)).toISOString().replace(".000Z", "Z");
const hours = (ns) => (Number(BigInt(ns) / 1_000_000_000n) / 3600).toFixed(1);

/** The words for one event. Pure, so it is tested without a network. */
export function describe(ev, status) {
  const [kind, e] = Object.entries(ev)[0];
  const head = `etched buyer timelock ${status?.timelock ?? ""}`.trim();
  if (kind === "Proposed") {
    const v = e.view;
    const lines = [`⚠️ ${head}: PROPOSAL ${v.id} (${v.kind})${v.by ? ` by ${v.by}` : ""}`,
      `can run from ${when(v.executable_ns)} (${hours(BigInt(v.executable_ns) - BigInt(v.proposed_ns))} h after it was proposed)`];
    if (opt(v.wasm_hash)) lines.push(`module sha256 ${opt(v.wasm_hash)} (${opt(v.wasm_bytes)} bytes)`);
    if (opt(v.arg_hash)) lines.push(`argument sha256 ${opt(v.arg_hash)}`);
    if (opt(v.controllers)) lines.push(`NEW CONTROLLERS for the buyer: ${opt(v.controllers).join(", ")}`);
    const r = opt(v.roles);
    if (r) lines.push(`NEW ROLES: proposers ${r.proposers.join(", ")}; guardians ${r.guardians.join(", ")}; veto ${r.veto_threshold} of ${r.guardians.length}`);
    lines.push(opt(v.wasm_hash)
      ? "Check it: build the published source with autobuyer/scripts/reproducible-build.sh and compare the hash. If it is not a build you expect, the guardians veto it."
      : r ? "A roles change replaces who may propose and who may veto. If it is not one you expect, the guardians veto it."
      : "A controller change moves the buyer to another controller. If it is not one you expect, the guardians veto it.");
    return lines.join("\n");
  }
  if (kind === "Cancelled") return `${head}: proposal ${e.id} WITHDRAWN by its proposer ${e.by} at ${when(e.at_ns)}`;
  if (kind === "VetoVote") return `${head}: guardian ${e.by} voted to VETO proposal ${e.id} (${e.votes} of ${e.needed})`;
  if (kind === "Vetoed") return `🛑 ${head}: proposal ${e.id} VETOED at ${when(e.at_ns)} by ${e.by.join(", ")}`;
  if (kind === "ResumeVote") return `${head}: guardian ${e.by} voted to RESUME the buyer (${e.votes} of ${e.needed})`;
  if (kind === "RolesChanged") return `⚠️ ${head}: proposal ${e.id} CHANGED THE ROLES: proposers ${e.roles.proposers.join(", ")}; guardians ${e.roles.guardians.join(", ")}; veto ${e.roles.veto_threshold}`;
  if (kind === "Executed") return `✅ ${head}: proposal ${e.id} EXECUTED by ${e.by} at ${when(e.at_ns)}. Verify the buyer: autobuyer/scripts/reproducible-build.sh verify <buyer> buyer ic`;
  if (kind === "Failed") return `${head}: proposal ${e.id} FAILED at ${when(e.at_ns)}: ${e.why}. Nothing changed; a retry is a new proposal and a new delay.`;
  if (kind === "Paused") return `${head}: the guardians ${e.paused ? "PAUSED" : "RESUMED"} the buyer at ${when(e.at_ns)}${e.ok ? "" : ` — the call FAILED: ${opt(e.why)}`}`;
  // audit D, L-2: `execute` stops the buyer, installs, and starts it again
  if (kind === "NotStopped") return `⚠️ ${head}: proposal ${e.id} did NOT run: the buyer did not stop (${e.why}). Nothing was installed; the buyer was started again and, if still wanted (not vetoed or withdrawn), the proposal is pending again and may be executed after 10 minutes.`;
  if (kind === "NotUploaded") return `⚠️ ${head}: proposal ${e.id} did NOT run: the module's chunks were not uploaded to the buyer's chunk store (${e.why}). Nothing was stopped or installed; if still wanted, the proposal is pending again and may be executed after 10 minutes.`;
  if (kind === "NotStarted") return `🚨 ${head}: proposal ${e.id} did not leave the buyer running: it did NOT START again (${e.why}). A stopped buyer answers nothing and delivers nothing. Any guardian can start it now (start_target, canister review2 N-6; recorded as Started); redemptions wait until then.`;
  // canister review2 N-6: a guardian's start_target
  if (kind === "Started") return `${e.ok ? "" : "🚨 "}${head}: guardian ${e.by} ${e.ok ? "STARTED the buyer" : "tried to start the buyer and it FAILED"} at ${when(e.at_ns)}${opt(e.why) ? ` (${opt(e.why)})` : ""}.${e.ok ? " A buyer that was paused stays paused (its own flag)." : " It is still stopped: nothing delivers."}`;
  return `${head}: an event this watcher does not know: ${kind}`;
}

/** What is wrong with the timelock's fixed configuration, against what the operator expects. Pure. */
export function surprises(status, expect, network) {
  const out = [];
  if (expect.target && status.target !== expect.target) out.push(`target is ${status.target}, expected ${expect.target}`);
  const same = (a, b) => JSON.stringify([...a].sort()) === JSON.stringify([...b].sort());
  const r = status.roles ?? {};
  if (expect.proposers && !same(r.proposers ?? [], expect.proposers)) out.push(`proposers are ${(r.proposers ?? []).join(", ")}, expected ${expect.proposers.join(", ")}`);
  if (expect.guardians && !same(r.guardians ?? [], expect.guardians)) out.push(`guardians are ${(r.guardians ?? []).join(", ")}, expected ${expect.guardians.join(", ")}`);
  if (expect.veto && String(r.veto_threshold) !== String(expect.veto)) out.push(`veto threshold is ${r.veto_threshold}, expected ${expect.veto}`);
  if (expect.delay && String(status.delay_secs) !== String(expect.delay)) out.push(`delay is ${status.delay_secs} s, expected ${expect.delay}`);
  if (network === "ic" && status.rehearsal_build) out.push("this is a REHEARSAL build (10-second floor) on the IC: it must never guard the real buyer");
  return out;
}

/** docs/v2/trust-points.md §2. For each canister, a message when it crosses its own published threshold (down or back up), and the
 *  new low flags. Pure: `was` is the saved flags, `now` maps a label to { balance, low_threshold } as `cycles()` returns them. */
export function cyclesAlerts(was, now) {
  const flags = {}, messages = [];
  const big = (x) => BigInt(String(x).replace(/_/g, ""));   // dfx's JSON writes a nat with digit separators: 3_500_000_000_000
  for (const [label, c] of Object.entries(now)) {
    const low = big(c.balance) < big(c.low_threshold);
    flags[label] = low;
    const t = (x) => `${(Number(big(x) / 1_000_000_000n) / 1000).toFixed(2)}T`;
    if (low && !was?.[label]) messages.push(`🔋 ${label} is LOW on cycles: ${t(c.balance)} left, below its ${t(c.low_threshold)} threshold. Anyone can top it up: dfx cycles top-up <id> <amount> --network ic`);
    if (!low && was?.[label]) messages.push(`🔋 ${label} is above its cycles threshold again: ${t(c.balance)}`);
  }
  return { flags, messages };
}

/** Post to Telegram, or print. The token goes in the URL path only and is scrubbed from any error. */
async function send(o, text) {
  const token = process.env.TELEGRAM_BOT_TOKEN, chat = process.env.TELEGRAM_CHAT_ID;
  if (o.dryRun || !token || !chat) { console.log(`--- ${o.dryRun ? "dry run" : "no TELEGRAM_BOT_TOKEN/TELEGRAM_CHAT_ID set"}: would post ---\n${text}\n`); return; }
  const base = process.env.TELEGRAM_API_BASE ?? "https://api.telegram.org";
  try {
    const r = await fetch(`${base}/bot${token}/sendMessage`, { method: "POST", headers: { "content-type": "application/json" },
      body: JSON.stringify({ chat_id: chat, text, disable_web_page_preview: true }), signal: AbortSignal.timeout(20_000) });
    if (!r.ok) throw new Error(`Telegram answered HTTP ${r.status}`);
    console.log(`posted: ${text.split("\n")[0]}`);
  } catch (err) {
    console.error(`could not post: ${String(err.message).split(token).join("<token>")}`);
    throw new Error("post failed");   // the event is not marked seen, so the next poll tries again
  }
}

async function poll(o) {
  const st = existsSync(o.state) ? JSON.parse(readFileSync(o.state, "utf8")) : {};
  const seen = st[o.timelock]?.seen ?? null;
  const [status, history] = await Promise.all([ask(o, "status"), ask(o, "history")]);
  status.timelock = o.timelock;
  const wrong = surprises(status, o.expect, o.network);
  if (wrong.length && JSON.stringify(wrong) !== JSON.stringify(st[o.timelock]?.wrong ?? [])) {
    await send(o, `🚨 etched buyer timelock ${o.timelock} is not configured as expected:\n- ${wrong.join("\n- ")}`);
  }
  if (seen === null) {
    // First look: say what is being watched, and anything pending now, without replaying the whole history as news.
    const pending = status.pending ?? [];
    await send(o, [`👀 watching the etched buyer timelock ${o.timelock} on ${o.network}`,
      `target ${status.target}; delay ${hours(BigInt(status.delay_secs) * 1_000_000_000n)} h; ${history.length} past events`,
      ...(pending.length ? pending.map((v) => describe({ Proposed: { view: v } }, status)) : ["nothing pending"])].join("\n"));
  } else {
    for (const ev of history.slice(seen)) await send(o, describe(ev, status));   // oldest first; stops at the first failure
  }
  // the cycles of both canisters, from their own public queries; a canister that does not answer is said once, not silently skipped
  const now = {}, missing = [];
  for (const [label, id, candid] of [[`the timelock ${o.timelock}`, o.timelock, CANDID], [`the buyer ${status.target}`, status.target, BUYER_CANDID]]) {
    try { now[label] = await ask(o, "cycles", id, candid); } catch (err) { missing.push(`${label}: ${err.message}`); }
  }
  const { flags, messages } = cyclesAlerts(st[o.timelock]?.low, now);
  for (const m of messages) await send(o, m);
  const unanswered = missing.length ? missing : [];
  if (unanswered.length && JSON.stringify(unanswered) !== JSON.stringify(st[o.timelock]?.unanswered ?? [])) {
    await send(o, `🔋 could not read cycles:\n- ${unanswered.join("\n- ")}`);
  }
  st[o.timelock] = { seen: history.length, wrong, low: { ...(st[o.timelock]?.low ?? {}), ...flags }, unanswered, at: new Date().toISOString() };
  writeFileSync(o.state, JSON.stringify(st, null, 2) + "\n");
  return { status, events: history.length, newEvents: seen === null ? 0 : history.length - seen };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  let o;
  try { o = parseArgs(process.argv.slice(2)); } catch (err) { console.error(`refused: ${err.message}`); process.exit(2); }
  for (;;) {
    try {
      const r = await poll(o);
      if (o.once) { console.log(`ok: ${r.events} events, ${r.newEvents} new`); break; }
    } catch (err) {
      console.error(`poll failed: ${err.message}`);
      if (o.once) process.exit(1);
    }
    await new Promise((r) => setTimeout(r, o.every * 1000));
  }
}
