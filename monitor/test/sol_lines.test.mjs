// W3 / W14b (docs/v9/canister-only-solana.md §13.1): the buyer's four Solana send lines and its own timer, decoded from the canister's own
// bytes (autobuyer/canister-rs/tests/fixtures/sol-lines.hex, written by tests/sol_lines.rs), paged per line: an incident on the desk line
// names its own clear and leaves the others alone, an insufficient_funds error on the pool line is SEV1, a step out long on the money line
// keeps phase B2's keys, and a silent timer pages unless switched off. No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const FIX = (f) => readFileSync(new URL(`../../autobuyer/canister-rs/tests/fixtures/${f}`, import.meta.url), "utf8").trim();

test("W3: the four lines decode from the canister's own bytes and each is paged on its own", async () => {
  const { IDL } = await import("@icp-sdk/core/candid");
  const { REPLY_TYPES, plain } = await import("../src/candid.mjs");
  const { solLinesAlerts, lineStepText, HOME_OUT_S, SOL_TIMER_SILENT_S } = await import("../src/ethnft.mjs");
  const reply = plain(IDL.decode(REPLY_TYPES.sol_lines, Uint8Array.from(Buffer.from(FIX("sol-lines.hex"), "hex"))));
  const [lines, timer, off] = reply;
  assert.deepEqual(lines.map(([id]) => Object.keys(id)[0]), ["Money", "Crank", "Desk", "Pool"]);
  assert.equal(off, false);
  const t = timer[0];
  assert.equal(Number(t.runs), 42);
  const byName = Object.fromEntries(lines.map(([id, l]) => [Object.keys(id)[0], l[0]]));
  // the desks' steps read in words
  assert.equal(lineStepText(byName.Desk.pending[0].step), "the mint/burn desk's mint for intent #9");
  assert.equal(lineStepText(byName.Pool.pending[0].step), "the pool desk's re-range #9");
  assert.equal(lineStepText(byName.Money.pending[0].step), "the custodian's Mark of ticket #9");
  // the signed bytes in flight are not public on any line
  for (const l of Object.values(byName)) if (l.line.saved.length) assert.equal(l.line.saved[0].tx?.length ?? 0, 0);
  const nowS = Number(t.last_run) + 60;
  const a = solLinesAlerts({ buyer: "b", reply, nowS });
  const desk = a.find((x) => x.key.startsWith("sol-line-desk-incident:b"));
  assert.ok(desk && desk.sev === "SEV1" && /sol_clear_line_incident\(variant \{ Desk \}\)/.test(desk.text) && /the other lines go on/.test(desk.text), JSON.stringify(a));
  const pool = a.find((x) => x.key.startsWith("sol-line-pool-preflight:b"));
  assert.ok(pool && pool.sev === "SEV1" && /Solana pool line/.test(pool.text), JSON.stringify(a));
  // the money line's step, out since its `at`, keeps phase B2's keys; nothing about the money line's incident (it has none)
  const at = Number(byName.Money.pending[0].at);
  const late = solLinesAlerts({ buyer: "b", reply, nowS: at + HOME_OUT_S });
  assert.ok(late.some((x) => x.key === `sol-line-out:b:${byName.Money.pending[0].intent}`), JSON.stringify(late));
  assert.ok(!a.some((x) => x.key.startsWith("sol-line-incident:")), "the money line has no incident");
  // the timer: silent for SOL_TIMER_SILENT_S pages SEV2; switched off it does not
  assert.ok(!a.some((x) => x.key.startsWith("sol-timer-silent")));
  const silent = solLinesAlerts({ buyer: "b", reply, nowS: Number(t.last_run) + SOL_TIMER_SILENT_S });
  assert.ok(silent.some((x) => x.key === "sol-timer-silent:b" && x.sev === "SEV2"), JSON.stringify(silent));
  const offReply = [lines, timer, true];
  assert.ok(!solLinesAlerts({ buyer: "b", reply: offReply, nowS: Number(t.last_run) + SOL_TIMER_SILENT_S }).some((x) => x.key.startsWith("sol-timer-silent")));
  // the crank line never used: nothing from it (the registry keeper's state it carried is retired with rook_core, D-l)
  assert.ok(!a.some((x) => x.key.startsWith("sol-line-crank")), JSON.stringify(a));
  // no reply (a buyer built before W3): nothing from here
  assert.deepEqual(solLinesAlerts({ buyer: "b", reply: null, nowS: 1 }), []);
});

test("W3: checkCanister reads sol_lines when the buyer has it, and the one line otherwise", async () => {
  const { checkCanister } = await import("../src/custodyhealth.mjs");
  const { IDL } = await import("@icp-sdk/core/candid");
  const { REPLY_TYPES, plain } = await import("../src/candid.mjs");
  const reply = plain(IDL.decode(REPLY_TYPES.sol_lines, Uint8Array.from(Buffer.from(FIX("sol-lines.hex"), "hex"))));
  const ic = { ticketLogs: async () => [], log: async () => [], ordProviders: async () => [["unisat", "u"]], holderDeposits: async () => [],
    solLine: async () => [], solLines: async () => reply };
  const now = (Number(reply[1][0].last_run) + 60) * 1000;
  const r = await checkCanister({ cfg: { buyer: "b" }, now, ic });
  assert.ok(r.alerts.some((x) => x.key.startsWith("sol-line-desk-incident:b")), JSON.stringify(r.alerts.map((x) => x.key)));
  // a buyer without sol_lines (its query fails): the one line, as before
  const old = { ...ic, solLines: async () => { throw new Error("no such method"); } };
  const r2 = await checkCanister({ cfg: { buyer: "b" }, now, ic: old });
  assert.ok(!r2.alerts.some((x) => x.key.startsWith("sol-line-desk")));
});
