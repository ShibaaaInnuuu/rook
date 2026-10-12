import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { test } from "node:test";
import { OFF_IN_PHASE1, activityFor, boardFor, isOff, navFor, offTargets, phaseFrom, policiesFor, sendRefusal, sendable, stackCollection, stackFrom, targetRefusal, type StackFile, type StackReader } from "../src/rook/phase.js";
import { PHASE1_TRUST, PHASE1_TRUST_LEDE } from "../src/rook/phase1Trust.js";
import { NAV } from "../src/rook/routes.js";
import { POLICIES } from "../src/rook/tiers.js";
import { securityTxt } from "../site.config.js";
import type { Activity, Board, BoardRow } from "../src/rook/apiTypes.js";

/** docs/v6/phase1-scope.md §8.3 (K5): the site's Phase-1 mode. These tests are its spec. */

const A = (n: number) => `0x${n.toString(16).padStart(40, "0")}` as `0x${string}`;
const STACK = {
  chainId: 4663, label: "stack", stage: 0, registry: A(1), owner: A(2), keeper: A(3), treasury: A(4), directory: A(5), poolManager: A(6),
  collections: [{ slug: "frogs", kind: "inscriptions", wrapper: A(0x10), factory: A(0x11), router: A(0x12), floorModule: A(0x13), lock: A(0x14), queue: A(0x13),
    mintDesk: A(0x15), raffleVault: A(0x16), burnVault: A(0x17), yoyoVault: A(0x18), offerBoard: A(0x19), wrapClaim: A(0x1a) }],
  caps: { stage: 0, mintsPerDayPerCollection: 3, itemsPerDayPerCollection: 3, custodyItemsPerCollection: 3, custodyWeiPerCollection: 10n ** 19n },
};
const COIN = A(0xc0), CURVE = A(0xcc), STRANGER = A(0xdead);

test("the switch: unset or empty is no phase, \"1\" is Phase 1, and anything else fails the build", () => {
  assert.equal(phaseFrom(undefined), null);
  assert.equal(phaseFrom(""), null);
  assert.equal(phaseFrom(" 1 "), 1);
  for (const v of ["2", "true", "p1", "phase1", "0"]) assert.throws(() => phaseFrom(v), /the only phase this build knows is 1/, v);
});

test("Phase 1 hides Bots and Rent: not in the nav, and their routes say why they are off", () => {
  const nav = navFor(NAV, true).map(([to]) => to);
  assert.ok(!nav.includes("bots"), "no Bots in the Phase-1 nav");
  assert.ok(!nav.includes("rent"));
  assert.deepEqual(navFor(NAV, false), NAV, "outside Phase 1 nothing is hidden");
  for (const at of ["bots", "rent"]) { assert.ok(isOff(at, true)); assert.ok(!isOff(at, false)); assert.ok(OFF_IN_PHASE1[at].why.length > 40); }
  for (const at of ["", "coins", "coin", "launch", "collections", "collection", "reserves", "redeem", "wallet", "docs", "trust"]) assert.ok(!isOff(at, true), `${at} stays on`);
  assert.ok(!isOff("constructor", true) && !isOff("toString", true), "only the table's own keys");
});

test("Phase 1 offers no YOYO policy in the launch wizard; raffle and burn stay", () => {
  assert.deepEqual(policiesFor(POLICIES, true).map((p) => p.id), ["raffle", "burn"]);
  assert.deepEqual(policiesFor(POLICIES, false).map((p) => p.id), ["raffle", "burn", "yoyo"]);
});

test("Phase 1 leaves pons coins (the Launcher path) off the board and the tape", () => {
  const row = (address: string, kind: "curve" | "pons") => ({ address, kind } as unknown as BoardRow);
  const b = { rows: [row(A(1), "curve"), row(A(2), "pons"), row(A(3), "curve")] } as unknown as Board;
  assert.deepEqual(boardFor(b, true).rows.map((r) => r.kind), ["curve", "curve"]);
  assert.equal(boardFor(b, false).rows.length, 3);
  const a = { items: [{ coin: { kind: "pons" } }, { coin: { kind: "curve" } }, { coin: null }] } as unknown as Activity;
  assert.equal(activityFor(a, true).items.length, 2);
  assert.equal(activityFor(a, false).items.length, 3);
});

test("the stack file: exactly one for 4663, labelled stack, with addresses, at stage 0, or the reason there is none", () => {
  assert.ok(stackFrom([STACK]).stack, "the good file");
  assert.match(stackFrom([]).why!, /No stack deployment file for chain 4663/);
  assert.match(stackFrom([{ ...STACK, label: undefined }]).why!, /No stack deployment file/, "the v1 deployment file is not a stack file");
  assert.match(stackFrom([{ ...STACK, chainId: 9466301 }]).why!, /No stack deployment file/, "the staging fork's stack is not 4663's");
  assert.match(stackFrom([STACK, STACK]).why!, /exactly one/);
  assert.match(stackFrom([{ ...STACK, owner: A(0) }]).why!, /owner is not an address/);
  assert.match(stackFrom([{ ...STACK, collections: [] }]).why!, /names no collection/);
  assert.match(stackFrom([{ ...STACK, collections: [{ ...STACK.collections[0], factory: "0x1234" }] }]).why!, /frogs factory is not an address/);
  assert.match(stackFrom([{ ...STACK, stage: 1, caps: { ...STACK.caps, stage: 1 } }]).why!, /stage 1; Phase 1 is stage 0/);
  assert.equal(stackCollection(stackFrom([STACK]).stack, "FROGS")?.factory, A(0x11));
  assert.equal(stackCollection(stackFrom([STACK]).stack, "apes"), null);
});

test("TxButton in Phase 1: no stack file, no send; another chain, no send", () => {
  assert.match(sendRefusal(stackFrom([]), 4663)!, /cannot send: No stack deployment file/);
  assert.equal(sendRefusal(stackFrom([STACK]), 4663), null);
  assert.match(sendRefusal(stackFrom([STACK]), 9466301)!, /not the stack's chain 4663/);
  assert.match(sendRefusal(stackFrom([STACK]), null)!, /reading chain none/);
});

test("the target check: the stack's own contracts yes; a coin or curve its factory launched yes (read on chain); the desk, the board, the YOYO vault and strangers no", async () => {
  const s = stackFrom([STACK]).stack as StackFile;
  const read: StackReader = {
    routerOf: async (f, coin) => (f.toLowerCase() === A(0x11) && coin.toLowerCase() === COIN ? CURVE : A(0)),
    tokenOf: async (a) => (a.toLowerCase() === CURVE ? COIN : a.toLowerCase() === STRANGER ? A(0xbad) : null),
  };
  for (const ok of [A(0x10), A(0x11), A(0x12), A(0x13), A(0x16), COIN, CURVE, CURVE.toUpperCase().replace("0X", "0x")]) assert.equal(await targetRefusal(s, ok, read), null, ok);
  assert.match((await targetRefusal(s, A(0x17), read))!, /not a contract of this deployment/, "the burn vault: the site sends nothing to it");
  assert.match((await targetRefusal(s, A(0x15), read))!, /frogs's mint desk, which is off in Phase 1/);
  assert.match((await targetRefusal(s, A(0x19), read))!, /offer board, which is off/);
  assert.match((await targetRefusal(s, A(0x18), read))!, /YOYO vault, which is off/);
  assert.match((await targetRefusal(s, STRANGER, read))!, /not a contract of this deployment/, "a curve-shaped contract whose token the factory never launched");
  assert.match((await targetRefusal(s, A(0xbeef), read))!, /not a contract of this deployment/);
  assert.match((await targetRefusal(s, "0x12", read))!, /no valid target/);
  const broken: StackReader = { routerOf: async () => { throw new Error("rpc down"); }, tokenOf: async () => { throw new Error("rpc down"); } };
  assert.match((await targetRefusal(s, COIN, broken))!, /not a contract of this deployment/, "an unreadable chain refuses, it does not wave through");
  // the raffle vault joined in wave 2 (fixes-W2-site.md §2): the keyless draw steps are sent to it; the burn vault is not a target
  assert.deepEqual([...sendable(s).keys()].sort(), [A(0x10), A(0x11), A(0x12), A(0x13), A(0x16)].sort());
  assert.deepEqual([...offTargets(s).keys()].sort(), [A(0x15), A(0x18), A(0x19)].sort());
});

test("the Trust page says §0 plainly: custody-only caps, uncapped curves (the operator's decision), not professionally audited, both devices, no pause on redemption", () => {
  const all = [PHASE1_TRUST_LEDE, ...PHASE1_TRUST.flatMap((s) => [s.title, ...s.body.flatMap((p) => [p.text, ...(p.list ?? [])])])].join("\n");
  const must = [
    /stage-0 caps bound custody only/,
    /They do not bound the ETH that sits in the contracts/,
    /number of open curves, and the ETH in them, is not capped/,
    /24 September 2026 whether to cap the number of open curves, and decided not to/,
    /AI-reviewed, not professionally audited/, /docs\/audit\/2026-09-26\/README\.md/, /\(G1\) is waived for stage 0 only/,
    /both of the operator's devices/, /ProtocolTreasury\.setShares and setOps/, /at once and with no delay/, /transfer ownership/, /72-hour public delay/, /mint unbacked wrapped units/,
    /at most the custody caps lost" \(75 ETH for the five Ethereum collections\) is true of custody only/,
    /15 ETH of value per Ethereum collection/, /no canister-wide total beyond the sum/, /move 25 ETH a day/,
    /Redemption cannot be paused/,
    /30 ETH at most/, /up to 40 items' worth/, /locked pools, which can never be withdrawn/,
  ];
  for (const m of must) assert.match(all, m);
  assert.doesNotMatch(all.replace(/not professionally audited|unaudited/g, ""), /\baudited\b|fully (safe|secure)|guarantee/i, "nothing reassuring the code does not back");
});

test("the Trust page states the roles and powers after the fixes (C-06, C-09; fixes-B-C §3), and never claims there is no withdraw path", () => {
  const all = [PHASE1_TRUST_LEDE, ...PHASE1_TRUST.flatMap((s) => [s.title, ...s.body.flatMap((p) => [p.text, ...(p.list ?? [])])])].join("\n");
  const must = [
    /the Safe \(2-of-2\) can take the whole ProtocolTreasury at once/, /allocated pots included/,          // C-06
    /cancel any draw before its round is pinned/, /pause any listing/, /pause and unpause the keeper/,
    /After 7 public days it can install a keeper of its own/,
    /The keeper is a hot key/, /±5% an hour and ±20% in any 24-hour window/, /about 1\.3 ETH a day for frogs/, /until someone pauses it/,   // C-09
    /can no longer drain the reserve's surplus through burns, record a delivery, choose a draw's snapshot block, re-roll a draw/,
    /The guardian can pause and cancel, nothing else/, /powers are pause and cancel only/,
    /7-day keeper rotation/, /pause the keeper at once/,
    /stale fallback/, /anyone can burn ETH-backed wrapped units at the backing per unit, less the base spread, with no floor price/, /holders can always burn at backing/,
    /published ticket list plus a cancel before the pin/, /there is no redraw/,
    /refundable by anyone 7 days after its pin/, /72 hours after its delivery deadline/,
    // R3 (fixes-R3-contracts §1.2, §2, §4, §5, §7)
    /from the moment the keeper is paused they pay the floor the keeper left or the backing per unit, whichever is lower, less the base spread/, /30 days after its deadline without that word/,
    /Wrapper\.markUndeliverable/, /a refund can never race a delivery/, /A public mint at the module may take room a closing buy was about to need/,
    /escape hatch 7 days after the close/,
    // docs/v6/relay-conversion.md §7: the new trust, said with its bound
    /No operator money buys frogs/, /Relay's solver and its settlement keys/, /one advance at a time, at most three a day/, /It never asks Relay's servers anything/,
  ];
  for (const m of must) assert.match(all, m);
  assert.doesNotMatch(all, /no withdraw path|absence of a withdraw path|redirects the treasury's income and its unallocated balance/i, "C-09: the false or understated claims are gone");
  assert.doesNotMatch(all, /burns never stop|Burns are never paused/i, "R3 B-10: the exact wording, not the old absolute");
  assert.deepEqual(PHASE1_TRUST.map((s) => s.id), ["caps", "curves", "audit", "safe", "keeper", "guardian", "rotation", "stale", "keyless", "relay", "ethnft", "redemption", "fueler"]);
  // docs/v7/cycles-from-fees.md §4.2 item 8: the ops share and the fueler's hot key, said with their bounds
  for (const re of [/ops share of the fees is operator income/, /hot key on money/, /only five kinds of transaction/, /at most 20 T cycles a run, 60 T in seven days/,
    /keeps at most 0.15 ETH/, /pages at the highest severity/, /None of this touches custody/]) assert.match(all, re);
  // the trust model rendered under it is the canister-only one: it states the trade (the canister's code, not Solana's validators,
  // enforces the backing), scopes "no transfer method" to the operator, names the delays, and never claims to be trustless or audited
  const model = readFileSync(new URL("../src/content/trust-model.md", import.meta.url), "utf8");
  assert.match(model, /Solana's validators enforce\s+nothing about the backing; the canister's code does/);
  assert.match(model, /move money by hand \(there is no transfer method\)/);
  assert.match(model, /every resume applies \*\*24 h\*\* after it is asked/);
  // operator decision 2026-10-12: the audit-firm line is gone; the model must still never claim an outside audit
  assert.doesNotMatch(model, /\b(independently|externally) audited\b|audited by (an? )?(independent|outside|external)/i);
  assert.doesNotMatch(model, /trustless(?! )|absence of a withdraw path|the Safe/);
});

test("no page offers the mint desk or sends to the offer board: the site's own code names neither as a transaction target", () => {
  const dirs = ["../src/pages/", "../src/ui/", "../src/rook/"];
  for (const d of dirs) for (const f of readdirSync(new URL(d, import.meta.url))) {
    if (!/\.tsx?$/.test(f) || f === "phase.ts") continue;
    // docs/v7/dormant-collections.md §3.3: the one `request` the site makes is the WakeDesk's (a bonded wake), never a mint desk's
    const src = readFileSync(new URL(d + f, import.meta.url), "utf8").replace(/abi:\s*WAKE_DESK,\s*functionName:\s*"request"/g, "abi: WAKE_DESK");
    assert.doesNotMatch(src, /address:\s*[^,}]*(mintDesk|offerBoard|yoyoVault)/, `${d}${f} sends to a desk, board or YOYO vault`);
    assert.doesNotMatch(src, /MINT_DESK|MintDesk\.request|functionName:\s*"(request|fulfill|submit|relist)"/, `${d}${f} names a desk or board call`);
  }
});

test("security.txt: none without a contact, RFC 9116 fields with one, and a bad contact fails the build", () => {
  assert.equal(securityTxt("", new Date(0)), null);
  const t = securityTxt("mailto:security@example.org", new Date("2026-09-25T00:00:00Z"), "https://rook.fun")!;
  assert.match(t, /^Contact: mailto:security@example.org\nExpires: 2027-09-25T00:00:00.000Z\nPreferred-Languages: en\nCanonical: https:\/\/rook.fun\/.well-known\/security.txt\n$/);
  assert.throws(() => securityTxt("security@example.org", new Date()), /not a mailto: or https: URL/);
});

test("docs/v7/phase1-nft.md: the Trust page says where Ethereum NFTs are held, what the canister checks, and who posts their floor", () => {
  const all = PHASE1_TRUST.map((s) => s.body.map((p) => [p.text, ...(p.list ?? [])].join(" ")).join(" ")).join(" ");
  for (const re of [/own threshold-key address on Ethereum/, /ownerOf on Ethereum/, /its own floor reading plus the collection's premium/, /final on Ethereum/,
    /The canister posts this collection's floor, not the keeper/, /holder deposits are on at launch/, /goes back to the address it came from/, /recent sales can only hold the floor back, never set it and never stop a post/, /its first post waits for three/, /a flood of transfers cannot hide a sale/, /the wrapper's 30-day late refund is off for that ticket/, /48 hours after it first arrived/, /the canister reads the collection's own sales on Ethereum itself/, /goes back to the module through Relay/, /held for the operator/, /refunded 72 hours after the deadline/]) assert.match(all, re);
});
