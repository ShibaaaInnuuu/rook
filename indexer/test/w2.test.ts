import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { test } from "node:test";
import { encodeAbiParameters, keccak256, type AbiEvent, toEventSelector, type Hex } from "viem";
import { guardedFetch, hostRefusal, isPublicIp, pinnedHttpsFetch, readCapped, Refused, TooLarge, urlRefusal, type Resolver } from "../src/outbound.js";
import { fetchPicture, getJson, isPublicHost, metadataOf } from "../src/rook/catalogSources.js";
import { foldLedger, LATE_DELIVERY_WINDOW_S, openState, PIN_TIMEOUT_S, UNRESOLVED } from "../src/wrapped/ledger.js";
import { checkPublished, foldDraws, judgeList, sentinelLeaf, ticketLeaf, tree, verifyProof } from "../src/wrapped/draws.js";
import { CURVE_READS, DRAW_EVENTS, FACTORY_READS, REGISTRY_READS, REGISTRY_SWITCH_EVENTS, WRAPPER_EVENTS } from "../src/wrapped/abis.js";
import { I_FLOOR_MODULE } from "../src/wrapped/floorModule.js";
import type { RaffleDraw } from "../types.js";

/** docs/audit/2026-09-26/fixes-W2-site.md §3: what the contract fixes changed for the indexer, and E-12's outbound guard. */

// ---- E-12 ---------------------------------------------------------------------------------------------------------------------

test("E-12: every private, loopback, link-local (metadata), CGNAT, ULA, multicast and reserved address is refused; public ones pass", () => {
  for (const ip of ["127.0.0.1", "10.0.0.5", "172.16.0.1", "172.31.255.255", "192.168.1.2", "169.254.169.254", "100.64.0.1", "100.127.255.254", "0.0.0.0",
    "192.0.0.8", "198.18.0.1", "224.0.0.1", "240.0.0.1", "255.255.255.255", "::1", "::", "fd12:3456::1", "fc00::1", "fe80::1", "ff02::1", "2001:db8::1",
    "::ffff:127.0.0.1", "::ffff:169.254.169.254", "64:ff9b::a00:5", "2002:a00:1::1", "::127.0.0.1", "fd00:ec2::254", "not-an-ip", "1.2.3", "300.1.1.1"]) {
    assert.equal(isPublicIp(ip), false, ip);
  }
  for (const ip of ["1.1.1.1", "8.8.8.8", "172.32.0.1", "100.128.0.1", "2606:4700:4700::1111", "::ffff:8.8.8.8", "2a00:1450:4001::200e"]) assert.equal(isPublicIp(ip), true, ip);
  for (const u of ["http://example.com/x", "https://localhost/x", "https://indexer/x", "https://a.internal/x", "https://printer.local/x", "https://10.0.0.5/x",
    "https://[::1]/x", "https://[fd12::1]/x", "https://100.64.1.1/x", "https://user:pw@example.com/x", "ftp://example.com/x", "not a url"]) assert.notEqual(hostRefusal(u), null, u);
  assert.equal(hostRefusal("https://i.seadn.io/x.png"), null);
  assert.equal(isPublicHost("100.64.0.1"), false, "CGNAT, which the old hostname check let through");
  assert.equal(isPublicHost("[fd12::1]"), false, "Railway's private IPv6, likewise");
});

const resolver = (map: Record<string, string[]>): Resolver => async (h) => { const a = map[h]; if (!a) throw new Error("ENOTFOUND"); return a.map((address) => ({ address, family: address.includes(":") ? 6 : 4 })); };

test("E-12: a public name that resolves to a private address is refused before any request (10.0.0.5.nip.io, a name pointing at fd12::)", async () => {
  const r = resolver({ "10.0.0.5.nip.io": ["10.0.0.5"], "evil.example": ["93.184.216.34", "fd12::7"], "ok.example": ["93.184.216.34"] });
  assert.match((await urlRefusal("https://10.0.0.5.nip.io/meta.json", r))!, /private or reserved address \(10\.0\.0\.5\)/);
  assert.match((await urlRefusal("https://evil.example/x", r))!, /fd12::7/, "one private answer among public ones is enough to refuse");
  assert.equal(await urlRefusal("https://ok.example/x", r), null);
  assert.match((await urlRefusal("https://nowhere.example/x", r))!, /does not resolve/);
  let asked = 0;
  const f = guardedFetch({ resolve: r, fetch: (async () => { asked++; return new Response("{}"); }) as typeof fetch });
  await assert.rejects(f("https://10.0.0.5.nip.io/meta.json"), Refused);
  assert.equal(asked, 0, "nothing was sent");
});

test("E-12: the production transport checks the address inside the socket's own lookup (no rebinding window): a name that resolves to loopback is refused at connect", async () => {
  // `localhost` resolves from the hosts file, with no network: the transport alone (without the pre-flight check) refuses it
  await assert.rejects(pinnedHttpsFetch("https://localhost:9/x"), /private or reserved address/);
  await assert.rejects(pinnedHttpsFetch("http://example.com/x"), /not https/);
});

test("E-12: redirects are followed by hand and re-checked: to a private host, to http, or a fourth hop is refused; an honest hop is followed", async () => {
  const r = resolver({ "a.example": ["93.184.216.34"], "b.example": ["93.184.216.35"], "meta.example": ["169.254.169.254"] });
  const hops: Record<string, Response | (() => Response)> = {
    "https://a.example/to-private": () => new Response(null, { status: 302, headers: { location: "https://meta.example/latest/meta-data/" } }),
    "https://a.example/to-http": () => new Response(null, { status: 301, headers: { location: "http://b.example/x" } }),
    "https://a.example/to-ip": () => new Response(null, { status: 307, headers: { location: "https://169.254.169.254/" } }),
    "https://a.example/ok": () => new Response(null, { status: 302, headers: { location: "/final" } }),
    "https://a.example/final": () => new Response('{"ok":true}', { status: 200 }),
    "https://a.example/loop": () => new Response(null, { status: 302, headers: { location: "https://a.example/loop" } }),
  };
  const seen: string[] = [];
  const transport = (async (u: string, init: RequestInit) => { seen.push(u); assert.equal(init.redirect, "manual", "the transport never follows a redirect itself"); const h = hops[u]; return typeof h === "function" ? h() : h; }) as unknown as typeof fetch;
  const f = guardedFetch({ resolve: r, fetch: transport });
  await assert.rejects(f("https://a.example/to-private"), /169\.254\.169\.254/);
  assert.ok(!seen.includes("https://meta.example/latest/meta-data/"), "the metadata host was never asked");
  await assert.rejects(f("https://a.example/to-http"), /not https/);
  await assert.rejects(f("https://a.example/to-ip"), /private or reserved/);
  await assert.rejects(f("https://a.example/loop"), /more than 3 redirects/);
  const ok = await f("https://a.example/ok");
  assert.deepEqual(await ok.json(), { ok: true });
});

test("E-12: bodies are read under a cap (with and without content-length), and the whole request has one deadline", async () => {
  const big = new Uint8Array(3 * 1024 * 1024);
  await assert.rejects(readCapped(new Response(big, { headers: { "content-length": String(big.byteLength) } }), 1024), TooLarge);
  const stream = new ReadableStream<Uint8Array>({ pull(c) { c.enqueue(new Uint8Array(64 * 1024)); } });   // endless, no length
  await assert.rejects(readCapped(new Response(stream), 1024 * 1024), TooLarge);
  const r = resolver({ "slow.example": ["93.184.216.34"], "big.example": ["93.184.216.34"] });
  const slow = guardedFetch({ resolve: r, timeoutMs: 50, fetch: ((_: string, init: RequestInit) => new Promise((_ok, no) => init.signal!.addEventListener("abort", () => no(init.signal!.reason)))) as unknown as typeof fetch });
  const alive = setTimeout(() => undefined, 1_000);   // AbortSignal.timeout's timer is unref'd: keep the loop up while it runs
  await assert.rejects(slow("https://slow.example/x"), /timeout|abort/i);
  clearTimeout(alive);
  const bigF = guardedFetch({ resolve: r, maxBytes: 1024, fetch: (async () => new Response(new ReadableStream<Uint8Array>({ pull(c) { c.enqueue(new Uint8Array(512)); } }))) as unknown as typeof fetch });
  await assert.rejects(bigF("https://big.example/x"), TooLarge);
  // getJson and fetchPicture read under their caps too, whatever transport they are given
  const endless = (async () => new Response(new ReadableStream<Uint8Array>({ pull(c) { c.enqueue(new TextEncoder().encode("[1,".repeat(10_000))); } }), { headers: { "content-type": "image/png" } })) as unknown as typeof fetch;
  assert.deepEqual(await getJson("https://api.example.com/x", endless), { ok: false, status: 200, error: "too large" });
  assert.equal((await fetchPicture("https://cdn.example.com/x.png", endless)).ok, false);
});

test("E-12: metadataOf and getJson refuse a private host before any request (they had no host check at all)", async () => {
  let asked = 0;
  const net = (async () => { asked++; return new Response("{}"); }) as unknown as typeof fetch;
  assert.equal(await metadataOf("https://10.0.0.5/meta.json", net), null);
  assert.equal(await metadataOf("https://169.254.169.254/latest/meta-data/", net), null);
  assert.equal(await metadataOf("http://example.com/meta.json", net), null);
  assert.match(String((await getJson("https://localhost/x", net) as { error?: string }).error), /refused/);
  assert.equal(asked, 0);
});

// ---- the wrapper's pinned refund (E-03) and the refund times ----------------------------------------------------------------------

test("a ticket refunded while pinned (acquisitionId = type(uint256).max) is refunded unresolved: no item goes back to the set; refund times follow the contract", () => {
  const ev = (name: string, args: Record<string, unknown>, block: number, ts = block) => ({ name, args, block, logIndex: 0, tx: `0x${block}` as `0x${string}`, ts });
  const l = foldLedger("inscriptions", [
    ev("Deposited", { frogId: 1n, inscriptionId: "x1i0", txid: "0x" + "a".repeat(64), vout: 0, to: "0xholder" }, 1),
    ev("Redeemed", { ticketId: 0n, owner: "0xo", round: 9n, destination: "bc1pdest" }, 2, 1_000),
    ev("Refunded", { ticketId: 0n, acquisitionId: (1n << 256n) - 1n, owner: "0xo" }, 3, 1_000 + PIN_TIMEOUT_S),
    ev("Redeemed", { ticketId: 1n, owner: "0xo", round: 10n, destination: "bc1pdest" }, 4, 2_000),
    ev("Redeemed", { ticketId: 2n, owner: "0xo", round: 11n, destination: "bc1pdest" }, 5, 3_000),
    ev("Resolved", { ticketId: 2n, acquisitionId: 1n, deliverBy: 50_000n }, 6, 3_100),
  ], { btcTx: (t) => `btc:${t}`, evmTx: null }, "BURN", 10_000);
  const t0 = l.tickets.get(0)!, t1 = l.tickets.get(1)!, t2 = l.tickets.get(2)!;
  assert.equal(UNRESOLVED, ((1n << 256n) - 1n).toString());
  assert.equal(t0.stage, "refunded"); assert.equal(t0.refundedUnresolved, true); assert.equal(t0.itemId, null); assert.equal(t0.refundableAt, null);
  assert.match(l.activity.find((a) => a.kind === "refunded")!.text, /not resolved within 7 days of its pin.*no item drawn/);
  assert.equal(t1.refundableAt, 2_000 + PIN_TIMEOUT_S, "a pinned ticket: 7 days after its pin");
  assert.equal(t2.refundableAt, 50_000 + LATE_DELIVERY_WINDOW_S, "a resolved ticket nobody attested: 30 days after deliverBy (R3 §4)");
  assert.equal(t2.undeliverable, false);
  assert.equal(t2.refundedUnresolved, false);
  assert.deepEqual(openState(l).held.map((i) => i.itemId), ["1"], "the one item is still held (promised to ticket 2)");
});

// ---- the draws -----------------------------------------------------------------------------------------------------------------

const A = (n: number) => `0x${n.toString(16).padStart(40, "0")}` as `0x${string}`;
const VAULT = A(0x16), COIN = A(0xc0);
const RANGES = [{ holder: A(0xa1), first: 0n, end: 5n }, { holder: A(0xa2), first: 5n, end: 12n }, { holder: A(0xa3), first: 12n, end: 20n }];
const SNAP = 1_900_000_000n, TOTAL = 20n;
const LEAVES = [...RANGES.map((r) => ticketLeaf(r.holder, r.first, r.end)), sentinelLeaf(TOTAL, SNAP)];
const ROOT = tree(LEAVES).root;
const LIST = { version: 2, root: ROOT, totalTickets: "20", entries: RANGES.map((r) => ({ holder: r.holder, balance: "1", first: String(r.first), end: String(r.end), leaf: ticketLeaf(r.holder, r.first, r.end), proof: [] })) };

test("the tree is SweepVault.leaf's: double-hashed abi.encode(holder, first, end); the sentinel is leaf(address(0), totalTickets, snapshotAt); proofs verify", () => {
  const manual = keccak256(keccak256(encodeAbiParameters([{ type: "address" }, { type: "uint256" }, { type: "uint256" }], [A(0xa1), 0n, 5n])));
  assert.equal(ticketLeaf(A(0xa1), 0n, 5n), manual);
  assert.equal(sentinelLeaf(TOTAL, SNAP), ticketLeaf(A(0), TOTAL, SNAP));
  const t = tree(LEAVES);
  for (const l of LEAVES) assert.ok(verifyProof(t.proofOf(l), t.root, l));
  assert.ok(!verifyProof(t.proofOf(LEAVES[0]), t.root, ticketLeaf(A(0xa1), 0n, 6n)), "a widened range does not verify");
});

const ev = (name: string, args: Record<string, unknown>, block: number, ts: number) => ({ name, args, block, logIndex: 0, tx: `0x${block.toString(16)}` as Hex, ts });
const drawEvents = () => [
  ev("DrawAnnounced", { drawId: 0n, coin: COIN, seedRound: 100n }, 1, 1_000),
  ev("SnapshotFixed", { drawId: 0n, snapshotAt: SNAP }, 2, 2_000),
  ev("DrawOpened", { drawId: 0n, coin: COIN, root: ROOT, totalTickets: TOTAL, uri: "https://lists.example/draw-0.json", challengeEnds: 2_900n }, 2, 2_000),
  ev("DrawPinned", { drawId: 0n, round: 500n }, 3, 3_000),
  ev("DrawResolved", { drawId: 0n, winningTicket: 7n }, 4, 4_000),
  ev("DrawAnnounced", { drawId: 1n, coin: COIN, seedRound: 900n }, 5, 5_000),
  ev("DrawCancelled", { drawId: 1n }, 6, 5_100),
  ev("DrawAnnounced", { drawId: 2n, coin: COIN, seedRound: 950n }, 7, 6_000),
];

test("the draws fold through every stage, newest first, with the times anyone may void them", () => {
  const ds = foldDraws(VAULT, drawEvents());
  assert.deepEqual(ds.map((d) => [d.id, d.stage]), [[2, "announced"], [1, "cancelled"], [0, "resolved"]]);
  const d0 = ds[2];
  assert.equal(d0.snapshotAt, Number(SNAP)); assert.equal(d0.root, ROOT); assert.equal(d0.totalTickets, "20"); assert.equal(d0.challengeEnds, 2_900);
  assert.equal(d0.round, 500); assert.equal(d0.pinnedAt, 3_000); assert.equal(d0.winningTicket, "7"); assert.equal(d0.resolvedAt, 4_000);
  assert.equal(d0.voidableAt, 4_000 + 7 * 86_400, "resolved and unclaimed: 7 days");
  assert.equal(ds[0].voidableAt, 6_000 + 6 * 3_600, "announced and never opened: 6 hours");
  assert.equal(ds[1].voidableAt, null, "a cancelled draw is final");
  const won = foldDraws(VAULT, [...drawEvents(), ev("Won", { drawId: 0n, holder: A(0xa2) }, 8, 7_000)]).find((d) => d.id === 0)!;
  assert.equal(won.stage, "claimed"); assert.equal(won.winner, A(0xa2)); assert.equal(won.voidableAt, null);
  const voided = foldDraws(VAULT, [...drawEvents(), ev("DrawVoided", { drawId: 2n }, 9, 30_000)]).find((d) => d.id === 2)!;
  assert.equal(voided.stage, "voided");
});

test("the published list: rebuilt with the sentinel it matches the root and yields the winner's claim; a truncated total or an edited list differs", () => {
  const d0 = foldDraws(VAULT, drawEvents()).find((d) => d.id === 0)! as RaffleDraw;
  const ok = judgeList(d0, LIST);
  assert.deepEqual(ok.published, { status: "matches", why: null });
  assert.equal(ok.claim!.holder, A(0xa2), "ticket 7 is in [5, 12)");
  assert.deepEqual([ok.claim!.first, ok.claim!.end], ["5", "12"]);
  assert.ok(verifyProof(ok.claim!.proof, ROOT, ticketLeaf(A(0xa2), 5n, 12n)));
  // C-03: the honest leaves with a truncated total rebuild to another root (the sentinel commits the count)
  assert.equal(judgeList({ ...d0, totalTickets: "12" }, LIST).published!.status, "differs");
  assert.equal(judgeList({ ...d0, snapshotAt: Number(SNAP) + 1 }, LIST).published!.status, "differs", "and the time");
  const edited = { ...LIST, entries: LIST.entries.map((e, i) => (i === 1 ? { ...e, holder: A(0xbad) } : e)) };
  assert.equal(judgeList(d0, edited).published!.status, "differs");
  assert.equal(judgeList(d0, edited).claim, null, "no claim is offered from a list that does not reproduce the root");
  assert.equal(judgeList(d0, { entries: [] }).published!.status, "unreadable");
  const open = { ...d0, stage: "open" as const, winningTicket: null };
  assert.equal(judgeList(open, LIST).claim, null, "no claim before the draw is resolved");
  // a sentinel written into the list as an entry is skipped, not counted twice
  assert.equal(judgeList(d0, { entries: [...LIST.entries, { holder: A(0), first: "20", end: String(SNAP) }] }).published!.status, "matches");
});

test("the published list is fetched under the E-12 rules: https or ipfs only, a private host refused, an oversized list refused", async () => {
  const d0 = foldDraws(VAULT, drawEvents()).find((d) => d.id === 0)!;
  const net = (async (u: string) => (u === "https://lists.example/draw-0.json" ? new Response(JSON.stringify(LIST)) : new Response("no", { status: 404 }))) as unknown as typeof fetch;
  assert.equal((await checkPublished(d0, net, [])).published!.status, "matches");
  let asked = 0;
  const count = (async () => { asked++; return new Response(JSON.stringify(LIST)); }) as unknown as typeof fetch;
  assert.match((await checkPublished({ ...d0, uri: "https://10.0.0.5/list.json" }, count, [])).published!.why!, /refused/);
  assert.match((await checkPublished({ ...d0, uri: "file:///etc/passwd" }, count, [])).published!.why!, /neither https nor ipfs/);
  assert.equal(asked, 0);
  const ipfs = (async (u: string) => (u === "https://gw.example/ipfs/bafy/list.json" ? new Response(JSON.stringify(LIST)) : new Response("", { status: 429 }))) as unknown as typeof fetch;
  assert.equal((await checkPublished({ ...d0, uri: "ipfs://bafy/list.json" }, ipfs, ["https://gw.example/ipfs/"])).published!.status, "matches");
  const huge = (async () => new Response(new ReadableStream<Uint8Array>({ pull(c) { c.enqueue(new Uint8Array(1 << 20)); } }))) as unknown as typeof fetch;
  assert.match((await checkPublished(d0, huge, [])).published!.why!, /larger than 8 MB/);
});

test("a draw whose list is on the R2 mirror (https://pub-<id>.r2.dev/rook-snapshots/…) passes the outbound guard and is fetched from the URI in the draw", async () => {
  const uri = "https://pub-736187aaa62744868846472f72e8d77b.r2.dev/rook-snapshots/snapshot-0x" + "c0".repeat(20) + "-123-weighted.json";
  const d0 = { ...foldDraws(VAULT, drawEvents()).find((d) => d.id === 0)!, uri };
  const cloudflare = async () => [{ address: "104.18.54.45", family: 4 }, { address: "2606:4700:311b::6812:3222", family: 6 }];
  assert.equal(hostRefusal(uri), null);
  assert.equal(await urlRefusal(uri, cloudflare), null);
  const asked: string[] = [];
  const transport = (async (u: string) => { asked.push(u); return new Response(JSON.stringify(LIST)); }) as unknown as typeof fetch;
  const guarded = guardedFetch({ resolve: cloudflare, fetch: transport, maxBytes: 8 * 1024 * 1024 });
  assert.equal((await checkPublished(d0, guarded, [])).published!.status, "matches");
  assert.deepEqual(asked, [uri], "the URI the draw carries, and nothing else");
  // the planned custom domain is the same rule
  assert.equal(await urlRefusal("https://snapshots.rook.fun/x.json", cloudflare), null);
});

// ---- the ABIs the indexer names, against forge's artifacts ------------------------------------------------------------------------

test("the wave-2 signatures are the contracts': draw and pause events, the curve's gates, the factory's launch gate, the module's surplus (skipped without out/)", () => {
  const out = new URL("../../out/", import.meta.url).pathname;
  if (!existsSync(`${out}SweepVault.sol/SweepVault.json`)) return;
  const abiOf = (c: string) => (JSON.parse(readFileSync(`${out}${c}.sol/${c}.json`, "utf8")) as { abi: { type: string; name?: string; inputs?: { type: string }[]; outputs?: { type: string }[]; stateMutability?: string }[] }).abi;
  const events = (c: string) => new Set(abiOf(c).filter((x) => x.type === "event").map((x) => toEventSelector(x as unknown as AbiEvent)));
  const sig = (x: { name?: string; inputs?: { type: string }[]; outputs?: { type: string }[] }) => `${x.name}(${(x.inputs ?? []).map((i) => i.type).join(",")})->(${(x.outputs ?? []).map((o) => o.type).join(",")})`;
  const fns = (c: string) => new Set(abiOf(c).filter((x) => x.type === "function").map(sig));
  for (const e of DRAW_EVENTS) assert.ok(events("SweepVault").has(toEventSelector(e as AbiEvent)), `SweepVault.${(e as AbiEvent).name}`);
  for (const e of REGISTRY_SWITCH_EVENTS) assert.ok(events("ListingRegistry").has(toEventSelector(e as AbiEvent)));
  for (const e of WRAPPER_EVENTS.filter((x) => (x as AbiEvent).name !== "Repinned")) assert.ok(events("Wrapper").has(toEventSelector(e as AbiEvent)), `Wrapper.${(e as AbiEvent).name}`);
  assert.ok(!abiOf("Wrapper").some((x) => x.name === "Repinned" || x.name === "redraw"), "the pinned round is final: no redraw, no Repinned (only the NFT wrapper, OFF, keeps them)");
  const check = (c: string, abi: readonly unknown[], names: string[]) => { for (const x of abi as { type: string; name: string }[]) if (x.type === "function" && names.includes(x.name)) assert.ok(fns(c).has(sig(x)), `${c}.${x.name}`); };
  check("FrogCurve", CURVE_READS, ["canClose", "canGraduate", "refundsOpenAt", "state", "progress"]);
  check("CurveFactory", FACTORY_READS, ["launchesOpen", "normalRaise", "earlyRaise", "earlyUntilEth", "maxRaise"]);
  check("ListingRegistry", REGISTRY_READS, ["keeperPaused", "guardian"]);
  check("FloorModule", I_FLOOR_MODULE, ["surplus", "escaped", "registry", "burnCapacity", "buffer"]);
});
