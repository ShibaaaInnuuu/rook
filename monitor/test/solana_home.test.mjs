// docs/v9/solana-migration.md §4 (phase B1): the buyer's upgrade argument may name a Solana home (`Init.home`). The monitor's strict decode
// knows every label of it (the `home` field, the `Solana` tag, its record's fields), on the canister's own bytes
// (autobuyer/canister-rs/tests/fixtures/init-solana.hex and init-dormant.hex, written by tests/dormant.rs), and its argument diff words a
// home that appears, or whose identity changes, as CRITICAL. No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { decodeInit, encodeInitPlain, INIT_FIELDS } from "../src/candid.mjs";
import { diffInit } from "../src/argdiff.mjs";

const FIX = (f) => readFileSync(new URL(`../../autobuyer/canister-rs/tests/fixtures/${f}`, import.meta.url), "utf8").trim();

test("an upgrade argument naming a Solana home decodes strictly: every label known", () => {
  const d = decodeInit(FIX("init-solana.hex"));
  assert.ok(d.ok, d.error);
  assert.deepEqual(d.unknown, []);
  const h = d.init.home[0].Solana;
  assert.equal(h.cluster, "mainnet-beta");
  assert.equal(h.rook_core, "books", "decision D-l: the books home, the only one an install accepts");
  assert.deepEqual(h.rook_backing, [], "rook_backing retired");
  assert.equal(h.nonce_seed, "rook-home-0");
  assert.deepEqual(h.blockhash_fallback, []);
  for (const f of ["home", "Solana", "cluster", "rook_core", "nonce_seed", "blockhash_fallback"]) assert.ok(INIT_FIELDS.includes(f), f);
  // the EVM-home argument carries the `home` label too (null), and decodes strictly
  const e = decodeInit(FIX("init-dormant.hex"));
  assert.ok(e.ok, e.error);
  assert.deepEqual(e.init.home, []);
});

test("a home that appears, or whose identity changes, is CRITICAL; its providers are a WARN", () => {
  const evm = decodeInit(FIX("init-dormant.hex")).init, sol = decodeInit(FIX("init-solana.hex")).init;
  const on = diffInit(evm, sol).lines;
  assert.ok(on.some((l) => l.sev === "CRITICAL" && /home: the EVM chain → Solana mainnet-beta/.test(l.text)), JSON.stringify(on));
  assert.ok(diffInit(sol, sol).unchanged.includes("home"));
  const moved = structuredClone(sol);
  moved.home[0].Solana.rook_core = "11111111111111111111111111111111";
  assert.ok(diffInit(sol, moved).lines.some((l) => l.sev === "CRITICAL" && /home\.rook_core/.test(l.text)));
  const rpc = structuredClone(sol);
  rpc.home[0].Solana.rpc_urls = ["https://a", "https://b"];
  const w = diffInit(sol, rpc).lines;
  assert.ok(w.some((l) => l.sev === "WARN" && /home\.rpc_urls/.test(l.text)) && !w.some((l) => l.sev === "CRITICAL"));
  // a plain argument without `home` still encodes (the test alerts' sample proposals) and decodes to no home
  const plain = { ...evm };
  delete plain.home;
  assert.deepEqual(decodeInit(encodeInitPlain(plain)).init.home, []);
});

test("the Solana home line's incident decodes from the canister's own bytes and pages SEV1; a refused preflight and a long step page too", async () => {
  const { IDL } = await import("@icp-sdk/core/candid");
  const { EthState, plain } = await import("../src/candid.mjs");
  const { ethNftAlerts, HOME_OUT_S } = await import("../src/ethnft.mjs");
  const st = plain(IDL.decode([EthState], Uint8Array.from(Buffer.from(FIX("eth-state-sol-incident.hex"), "hex")))[0]);
  const sol = st.sol[0];
  assert.ok(sol.line.incident[0].includes("the nonce account moved"), JSON.stringify(sol.line));
  assert.equal(Object.keys(sol.pending[0].step)[0], "Advance");
  const at = Number(sol.pending[0].at);
  const now = at + 60;
  const list = ethNftAlerts({ buyer: "b", state: st, nowS: now }).alerts;
  const inc = list.find((x) => x.key.startsWith("eth-sol-incident"));
  assert.ok(inc && inc.sev === "SEV1" && /sol_clear_incident/.test(inc.text), JSON.stringify(list));
  // a preflight refused for want of funds is SEV1 and names sol_address; an instruction refused is SEV2
  const funds = structuredClone(st);
  funds.sol[0].line.incident = [];
  funds.sol[0].line.error = ["intent 1 (advance (buy 0)): every provider's preflight refused it: insufficient_funds"];
  const f = ethNftAlerts({ buyer: "b", state: funds, nowS: now }).alerts;
  assert.ok(f.some((x) => x.key.startsWith("eth-sol-preflight") && x.sev === "SEV1" && /sol_address/.test(x.text)), JSON.stringify(f));
  // out past 30 min: SEV2
  const late = ethNftAlerts({ buyer: "b", state: st, nowS: at + HOME_OUT_S }).alerts;
  assert.ok(late.some((x) => x.key.startsWith("eth-sol-out") && x.sev === "SEV2"), JSON.stringify(late));
  // an EVM-home state has no `sol`: nothing pages
  const evm = plain(IDL.decode([EthState], Uint8Array.from(Buffer.from(FIX("eth-state-dormant.hex"), "hex")))[0]);
  const e = ethNftAlerts({ buyer: "b", state: evm, nowS: 1 }).alerts;
  assert.ok(!e.some((x) => x.key.startsWith("eth-sol-")));
});

test("phase B2: the canister-wide Solana line decodes from the canister's own bytes and pages its incident, its refused simulation, a step out and a turn held", async () => {
  const { IDL } = await import("@icp-sdk/core/candid");
  const { REPLY_TYPES, plain } = await import("../src/candid.mjs");
  const { solLineAlerts, lineStepText, HOME_OUT_S } = await import("../src/ethnft.mjs");
  // tests/fixtures/sol-line-incident.hex: `sol_line()` with an incident under the Ethereum side's pending advance (tests/ethnft_sol.rs)
  const [opt] = IDL.decode(REPLY_TYPES.sol_line, Uint8Array.from(Buffer.from(FIX("sol-line-incident.hex"), "hex")));
  const line = plain(opt)[0];
  assert.ok(line.line.incident[0].includes("the nonce account moved"), JSON.stringify(line.line));
  assert.equal(lineStepText(line.pending[0].step), "the Ethereum side's Advance of buy #0");
  const at = Number(line.pending[0].at);
  const inc = solLineAlerts({ buyer: "b", line, nowS: at + 60 });
  assert.ok(inc.some((x) => x.key.startsWith("sol-line-incident:b") && x.sev === "SEV1" && /sol_clear_incident/.test(x.text)), JSON.stringify(inc));
  // a simulation every provider refused for want of funds: SEV1, names sol_address; an instruction refused: SEV2
  const funds = structuredClone(line);
  funds.line.incident = [];
  funds.line.error = ["intent 3 (frogs: advance (frog 0)): simulation refused by every provider: insufficient_funds \"AccountNotFound\"; nothing was signed"];
  assert.ok(solLineAlerts({ buyer: "b", line: funds, nowS: at + 60 }).some((x) => x.key.startsWith("sol-line-preflight") && x.sev === "SEV1" && /sol_address/.test(x.text)));
  // a step out 30 min: SEV2; a flow waiting its turn that long: SEV2
  const late = solLineAlerts({ buyer: "b", line: funds, nowS: at + HOME_OUT_S });
  assert.ok(late.some((x) => x.key.startsWith("sol-line-out") && x.sev === "SEV2"), JSON.stringify(late));
  const queued = structuredClone(funds);
  queued.queue = [[{ Frog: null }, BigInt(at)]];
  assert.ok(solLineAlerts({ buyer: "b", line: queued, nowS: at + HOME_OUT_S }).some((x) => x.key.startsWith("sol-line-queue:b:Frog")));
  // the other flows' steps read in words
  assert.equal(lineStepText({ Frog: { Convert: { frog: 4n } } }), "the frog side's Convert of frog #4");
  assert.equal(lineStepText({ Custody: { Record: { ticket: 2n, txid: "ab" } } }), "the custodian's Record of ticket #2");
  assert.equal(lineStepText({ Frog: { Post: null } }), "the frog side's Post");
  // no line (an EVM-home buyer): nothing
  assert.deepEqual(solLineAlerts({ buyer: "b", line: null, nowS: 1 }), []);
});

test("phase B2: a frog redemption's record and mark out on the Solana line decode, and a mark out a day pages as late", async () => {
  const { IDL } = await import("@icp-sdk/core/candid");
  const { REPLY_TYPES, plain } = await import("../src/candid.mjs");
  const { judgeCanister } = await import("../src/custodyhealth.mjs");
  // tests/fixtures/redemptions-sol.hex (tests/frogs_sol.rs): record_delivery and mark_undeliverable out on the canister-wide line
  const rs = plain(IDL.decode(REPLY_TYPES.redemptions, Uint8Array.from(Buffer.from(FIX("redemptions-sol.hex"), "hex")))[0]);
  assert.equal(Object.keys(rs[0].phase)[0], "SolRecording");
  assert.ok(rs[0].phase.SolRecording.intent !== undefined);
  assert.equal(Object.keys(rs[1].attest[0])[0], "SolSent");
  const read = { ticketLogs: [], log: [], ordProviders: [["unisat", "u"]], holderDeposits: [], feePool: null, redemptions: rs };
  const first = judgeCanister({ buyer: "b", read, now: 0 });
  const later = judgeCanister({ buyer: "b", read, prev: first.next, now: 25 * 3_600_000 });
  assert.ok(later.alerts.some((x) => x.key === `attest-late:b:${rs[1].ticket}` && /SolSent/.test(x.text)), JSON.stringify(later.alerts));
});
