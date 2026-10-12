import assert from "node:assert/strict";
import { test } from "node:test";
import { IDL } from "@icp-sdk/core/candid";
import { canisterIdl, decodeOpenQueue, encodeOpenQueue, type CollectionViewV, type OpenStatusV } from "../src/rook/sol/canister";
import { configOf, openingOf, queueStatusOf, requestCopy, waitingToOpen } from "../src/rook/sol/opening";

// Operator decision 2026-10-12 ("we don't select, the people do"): a pinned collection opens when someone first asks (coins_request_open);
// the site reads its state from coins_collections by wX symbol and launches under the canister's DBC config once it is Open.
const row = (index: number, symbol: string, stage: string, config = "", wx_mint = ""): CollectionViewV =>
  ({ index, symbol, stage, wx_mint, config, module: "", coins: 0, backed: 0 });
const COLS = [row(0, "wBOREDAPE", "Open", "Cfg1111111111111111111111111111111111111111", "Mint111111111111111111111111111111111111111"),
  row(2, "wMADLAD", "Pinned"), row(3, "wSMBGEN2", "Requested"), row(4, "wBFROG", "Opening"), row(5, "wPUDGY", "Refused(\"its config differs\")")];

test("a collection's opening, from coins_collections by its wX symbol", () => {
  assert.deepEqual(openingOf(undefined, "wMADLAD"), { kind: "unknown" });
  assert.deepEqual(openingOf(COLS, "wOMB"), { kind: "not-pinned" });
  assert.deepEqual(openingOf(COLS, "wMADLAD"), { kind: "pinned", index: 2 });
  assert.deepEqual(openingOf(COLS, "wSMBGEN2"), { kind: "requested", index: 3 });
  assert.deepEqual(openingOf(COLS, "wBFROG"), { kind: "opening", index: 4 });
  assert.deepEqual(openingOf(COLS, "wPUDGY"), { kind: "refused", index: 5, why: "its config differs" });
  assert.deepEqual(openingOf(COLS, "wBOREDAPE"), { kind: "open", index: 0, config: COLS[0].config, wxMint: COLS[0].wx_mint });
  // only a request in flight polls fast
  assert.deepEqual(["wMADLAD", "wSMBGEN2", "wBFROG", "wBOREDAPE"].map((s) => waitingToOpen(openingOf(COLS, s))), [false, true, true, false]);
});

test("the config a coin launches under: the site config's, else the canister's once Open; none before", () => {
  assert.equal(configOf("SiteCfg", openingOf(COLS, "wMADLAD")), "SiteCfg");
  assert.equal(configOf(null, openingOf(COLS, "wBOREDAPE")), COLS[0].config);
  assert.equal(configOf(null, openingOf(COLS, "wMADLAD")), null, "pinned, not open: no config, no launch");
  assert.equal(configOf("", openingOf(COLS, "wSMBGEN2")), null);
});

test("the words: 'Not open yet: be the first' with 'Open it' only while it can be asked", () => {
  const p = requestCopy(openingOf(COLS, "wMADLAD"), "Mad Lads");
  assert.equal(p.title, "Not open yet: be the first");
  assert.equal(p.action, "Open it");
  assert.match(p.body, /at most 8 a day/);
  assert.doesNotMatch(p.body, /a few minutes/, "audit C4-12: no promise of minutes before it is asked");
  for (const s of ["wSMBGEN2", "wBFROG", "wPUDGY", "wOMB"]) assert.equal(requestCopy(openingOf(COLS, s), "x").action, null, s);
});

// Audit C4-12: a requested collection's words are the canister's truth (coins_open_queue), not "a few minutes" whatever the queue does.
test("audit C4-12: the queue's place and, past 5 minutes of waiting, since when and why; the IDL round-trips coins_open_queue", () => {
  const now = 1_800_000_000;
  const st = (position: number, waiting_since = 0n, why = "", skipped = false): OpenStatusV => ({ index: 3, position, asked_at: BigInt(now - 900), waiting_since, why, skipped });
  const req = openingOf(COLS, "wSMBGEN2");
  assert.equal(queueStatusOf([st(0)], req)?.position, 0);
  assert.equal(queueStatusOf([{ ...st(0), index: 9 }], req), null, "another collection's entry");
  assert.equal(queueStatusOf([st(0)], openingOf(COLS, "wMADLAD")), null, "not asked for: no queue line");
  // next, not waiting: it opens next, a few minutes
  assert.match(requestCopy(req, "SMB Gen2", "Solana", st(0), now).body, /It opens next\. It should take a few minutes\./);
  // behind others: how many, at the pace of 8 a day, no minutes promised
  const behind = requestCopy(req, "SMB Gen2", "Solana", st(3), now).body;
  assert.match(behind, /3 requests are ahead of it; Rook opens at most 8 a day\./);
  assert.doesNotMatch(behind, /few minutes/);
  assert.match(requestCopy(req, "SMB Gen2", "Solana", st(1), now).body, /1 request is ahead of it/);
  // a short wait is an opening in progress; past 5 minutes the page says it waits, since when, and the canister's reason
  assert.doesNotMatch(requestCopy(req, "SMB Gen2", "Solana", st(0, BigInt(now - 120), "the float"), now).body, /Waiting/);
  const since = now - 40 * 60, d = new Date(since * 1000), hhmm = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  const w = requestCopy(req, "SMB Gen2", "Solana", st(0, BigInt(since), "the float is below the reserve plus one opening."), now).body;
  assert.equal(w, `SMB Gen2 is in the queue to open. It opens next. Waiting since ${hhmm}: the float is below the reserve plus one opening.`);
  assert.match(requestCopy(req, "x", undefined, st(2, BigInt(now - 3 * 86_400), "a pinned mint the chain lacks", true), now).body,
    /Waiting for 3 days: a pinned mint the chain lacks\. Later requests go first meanwhile; it keeps its place\./);
  assert.match(requestCopy(req, "x", undefined, st(0, BigInt(since)), now).body, /: no reason given\./);
  // an Opening collection whose pass waits says so too; without a queue entry the page promises nothing
  assert.match(requestCopy(openingOf(COLS, "wBFROG"), "Bitcoin Frogs", "Bitcoin", { ...st(0, BigInt(since), "the float"), index: 4 }, now).body, /Waiting since/);
  assert.equal(requestCopy(req, "SMB Gen2", "Solana", null, now).body, "SMB Gen2 is in the queue to open.");
  // the IDL: coins_open_queue's records as the canister encodes them decode to the same values, and the method is a query
  const q = [st(0, BigInt(since), "the float"), { ...st(1), index: 7, skipped: true }];
  const hex = Array.from(encodeOpenQueue(q), (b) => b.toString(16).padStart(2, "0")).join("");
  assert.deepEqual(decodeOpenQueue(hex), q);
  const svc = canisterIdl({ IDL }) as unknown as IDL.ServiceClass;
  const m = svc._fields.find(([n]) => n === "coins_open_queue");
  assert.ok(m, "coins_open_queue is in the IDL");
  assert.deepEqual(m![1].annotations, ["query"]);
});
