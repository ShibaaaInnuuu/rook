import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { encodeFunctionResult, parseAbi, decodeFunctionData, type Hex } from "viem";
import type { ReserveReport } from "../../indexer/types.js";
import { checkInscription } from "../../indexer/src/wrapped/verify.js";
import { Telegram } from "../../monitor/src/alerts.mjs";
import { backingStatus, readBacking, type Backing, type BackingRead } from "../src/backing.js";
import { agreeingOrdFetch } from "../src/location.js";
import { alert, bounded, combine, runOnce, served, Store, testAlerts, type Channel, type ReconcileConfig } from "../src/reconcile.js";
import { compareCaps, type CapsRead } from "../src/stage.js";

/**
 * Wave 2 (docs/audit/2026-09-26/fixes-W2-monitor.md §2): the reconciler holds the module's real ETH to its books (E-05), believes
 * an inscription's location only from two ordinals providers agreeing (E-13), pages when it cannot read or stays unverified
 * (E-08a), keeps its watchdog apart from a hung run, and refuses to start without Telegram (E-08b). The audit's E-08 PoCs,
 * inverted. No network.
 */
const E = 10n ** 18n, NOW_S = 1_790_000_000;
const FROGS = JSON.parse(readFileSync(new URL("../../indexer/fixtures/wrapped/reserves-frogs.json", import.meta.url), "utf8")) as ReserveReport;
const APES = JSON.parse(readFileSync(new URL("../../indexer/fixtures/wrapped/reserves-apes.json", import.meta.url), "utf8")) as ReserveReport;
const mod = (o: Partial<Omit<Backing, "books">> & { books?: Partial<Backing["books"]> } = {}): Backing => {
  const books = { backing: 2n * E, reserve: E / 2n, surplus: E / 10n, revenue: 0n, gradEth: 0n, rentalEscrow: 0n, renterPool: 0n, owedTotal: 0n, ...(o.books ?? {}) };
  return { slug: "frogs", module: "0xbb", wrapper: "0xaa", block: 100, balance: 3n * E, owed: Object.values(books).reduce((s, x) => s + x, 0n), reserveUnits: E,
    ethBacked: 4n * E, accepted: 65n * E / 100n, fresh: true, escaped: false, postedAt: NOW_S - 600, lastGoodAt: NOW_S - 600, floorTtl: 3600, ...o, books };
};
const read = (...modules: Backing[]): BackingRead => ({ kind: "read", at: NOW_S, modules });

test("E-05: green when the module holds its books, backs its units and is fresh; red when it does not", () => {
  assert.deepEqual(backingStatus(read(mod()), NOW_S), { status: "green", reasons: [] });
  const short = backingStatus(read(mod({ balance: 2n * E })), NOW_S);
  assert.equal(short.status, "red");
  assert.match(short.reasons[0], /holds 2 ETH, less than it owes by its own books \(2\.6 ETH: backing 2 ETH \+ reserve 0\.5 ETH \+ surplus 0\.1 ETH/);
  assert.match(backingStatus(read(mod({ reserveUnits: 5n * E })), NOW_S).reasons[0], /counts 5000000000000000000 reserve units but the wrapper has only 4000000000000000000/);
  const thin = backingStatus(read(mod({ ethBacked: 10n * E })), NOW_S);                     // 2.5 ETH for 10 units = 0.25 a unit, under half of 0.65
  assert.match(thin.reasons[0], /escrow 2\.5 ETH backs 10 ETH units at 0\.25 ETH a unit, under 50% of the floor 0\.65 ETH/);
  assert.equal(backingStatus(read(mod({ ethBacked: 10n * E })), NOW_S, { minBackingBps: 3_000 }).status, "green", "the fraction is RECONCILE_MIN_BACKING_BPS");
  assert.equal(backingStatus(read(mod({ fresh: false, postedAt: NOW_S - 3600 - 1800, lastGoodAt: NOW_S - 3600 - 1800 })), NOW_S).status, "green", "stale for half an hour: not yet");
  assert.match(backingStatus(read(mod({ fresh: false, postedAt: NOW_S - 3600 - 3601, lastGoodAt: NOW_S - 3600 - 3601 })), NOW_S).reasons[0], /the floor is stale since/);
  assert.match(backingStatus(read(mod({ fresh: false, escaped: true })), NOW_S).reasons[0], /in the stale fallback \(escaped\)/);
  assert.equal(backingStatus(read(mod({ fresh: false, postedAt: 0, lastGoodAt: 0, ethBacked: 0n, reserveUnits: 0n, books: { backing: 0n, reserve: 0n } })), NOW_S).status, "green",
    "a module that never posted and backs nothing: a stack before its launch");
  assert.equal(backingStatus({ kind: "error", at: 1, error: "rpc down" }, NOW_S).status, "unverified");
  // and it counts in the whole: a green reserves report with a short module is RED
  const caps = compareCaps({ collections: [{ collection: "btc:0x0", max_items: 3, max_value_wei: 10n * E, max_item_wei: 10n * E }], global_max_items: 6, global_max_value_wei: 10n * E }, [], 0, 1);
  assert.equal(combine([FROGS], caps, NOW_S, 4663, read(mod({ balance: E }))).status, "red");
  assert.equal(combine([FROGS], caps, NOW_S, 4663, read(mod())).status, "green");
});

test("E-05: readBacking reads every book, the balance and the units at ONE block", async () => {
  const MOD = parseAbi(["function backing() view returns (uint256)", "function reserve() view returns (uint256)", "function surplus() view returns (uint256)",
    "function revenue() view returns (uint256)", "function gradEth() view returns (uint256)", "function rentalEscrow() view returns (uint256)", "function renterPool() view returns (uint256)",
    "function owedTotal() view returns (uint256)", "function reserveUnits() view returns (uint256)", "function accepted() view returns (uint256)", "function fresh() view returns (bool)",
    "function escaped() view returns (bool)", "function postedAt() view returns (uint64)", "function lastGoodAt() view returns (uint64)", "function FLOOR_TTL() view returns (uint256)",
    "function ethBacked() view returns (uint256)"]);
  const values: Record<string, unknown> = { backing: 2n * E, reserve: E, surplus: 3n * E, revenue: 0n, gradEth: 0n, rentalEscrow: 0n, renterPool: 0n, owedTotal: 0n, reserveUnits: E,
    accepted: E / 2n, fresh: true, escaped: false, postedAt: BigInt(NOW_S), lastGoodAt: BigInt(NOW_S), FLOOR_TTL: 3600n, ethBacked: 3n * E };
  const tags = new Set<string>();
  const rpc = async <T>(method: string, params: unknown[]): Promise<T> => {
    if (method === "eth_blockNumber") return "0x3e8" as T;
    const tag = params[1] as string; tags.add(tag);
    if (method === "eth_getBalance") return `0x${(6n * E).toString(16)}` as T;
    const { functionName } = decodeFunctionData({ abi: MOD, data: (params[0] as { data: Hex }).data });
    return encodeFunctionResult({ abi: MOD, functionName, result: values[functionName] } as never) as T;
  };
  const b = await readBacking(rpc, { collections: [{ slug: "frogs", wrapper: "0x" + "aa".repeat(20), floorModule: "0x" + "bb".repeat(20) }] }, NOW_S);
  assert.equal(b.kind, "read");
  const m = (b as { modules: Backing[] }).modules[0];
  assert.deepEqual([m.block, m.balance, m.owed, m.books.surplus, m.ethBacked], [980, 6n * E, 6n * E, 3n * E, 3n * E]);
  assert.deepEqual([...tags], ["0x3d4"], "every read at the same block, 20 under the head");
  assert.equal(backingStatus(b, NOW_S).status, "green");
  const err = await readBacking(async () => { throw new Error("no RPC reports chain id 4663"); }, { collections: [{ slug: "f", wrapper: "0x" + "aa".repeat(20), floorModule: "0x" + "bb".repeat(20) }] }, NOW_S);
  assert.deepEqual([err.kind, backingStatus(err, NOW_S).status], ["error", "unverified"]);
});

// ---- E-13 ---------------------------------------------------------------------------------------------------------------------

const ID = "b6ceea8eb7e46c5784c6ff3eb3af6e5eca91f53ed8a06cdda7fe6f6c1b5f738ei0", TXID = ID.slice(0, 64);
const CUSTODY = "bc1pfssfalzdt9h8uwqcrs73780l723mh2axrul7keaevke7a8pg4a7szrd0ht";
function bitcoin(ord: { address: string; satpoint: string }, ow: { address: string; satpoint: string } | number) {
  const asked: string[] = [];
  const json = (v: unknown, status = 200) => new Response(JSON.stringify(v), { status, headers: { "content-type": "application/json" } });
  const f = (async (input: string | URL | Request) => {
    const url = String(input); asked.push(url);
    if (url.startsWith("https://ordinals.com/r/inscription/")) return json({ id: ID, ...ord });
    if (url.startsWith("https://turbo.ordinalswallet.com/inscription/")) return typeof ow === "number" ? json({}, ow) : json({ id: ID, ...ow });
    if (url.endsWith(`/tx/${TXID}`)) return json({ vout: [{ scriptpubkey_address: CUSTODY }], status: { confirmed: true, block_height: 900_000 } });
    if (url.endsWith(`/tx/${TXID}/outspend/0`)) return json({ spent: false });
    return json({}, 404);
  }) as typeof fetch;
  return { f, asked };
}
const at = (address: string) => ({ address, satpoint: `${TXID}:0:0` });

test("E-13: an inscription is in custody only when ord AND Ordinals Wallet agree; one provider alone can make it neither green nor red", async () => {
  const src = (f: typeof fetch) => ({ fetch: agreeingOrdFetch(f), now: () => NOW_S });
  const both = bitcoin(at(CUSTODY), at(CUSTODY));
  assert.equal((await checkInscription(ID, CUSTODY, src(both.f))).verdict, "in-custody");
  assert.ok(both.asked.some((u) => u.startsWith("https://turbo.ordinalswallet.com/inscription/")), "the second provider was asked");
  const liar = bitcoin(at(CUSTODY), at("bc1pelsewhere"));
  const v = await checkInscription(ID, CUSTODY, src(liar.f));
  assert.equal(v.verdict, "unverified", "ord alone cannot keep a moved frog green");
  assert.match(v.reason!, /disagree/);
  const redOnDemand = bitcoin(at("bc1pelsewhere"), at(CUSTODY));
  assert.equal((await checkInscription(ID, CUSTODY, src(redOnDemand.f))).verdict, "unverified", "nor make the page red on demand");
  assert.equal((await checkInscription(ID, CUSTODY, src(bitcoin(at(CUSTODY), 503).f))).verdict, "unverified", "the second provider down: unverified");
  assert.equal((await checkInscription(ID, CUSTODY, src(bitcoin(at("bc1pelsewhere"), at("bc1pelsewhere")).f))).verdict, "elsewhere", "both agreeing it moved: elsewhere");
  // every other request passes through untouched
  const pass = bitcoin(at(CUSTODY), at(CUSTODY));
  await agreeingOrdFetch(pass.f)("https://mempool.space/api/tx/" + TXID);
  assert.deepEqual(pass.asked, ["https://mempool.space/api/tx/" + TXID]);
});

// ---- E-08 --------------------------------------------------------------------------------------------------------------------

const cfg = (dir: string): ReconcileConfig => ({ stackPath: "", chainId: 4663, rpcUrls: [], canisterId: "aaaaa-aaaaa-aaaaa-aaaaa-aaa", icHost: "", everyS: 3_600, staleS: 10_800,
  realertS: 21_600, stateDir: dir, port: null, publicUrl: null, unverifiedAlertS: 21_600 });
const capsDown = async (): Promise<CapsRead> => ({ kind: "error", error: "IC unreachable", stackStage: 0, at: 1 });

test("E-08a regression: a reconciler that reads nothing pages at once ('cannot read'), and again every 6 h — not a silent week", async () => {
  const dir = mkdtempSync(join(tmpdir(), "w2-rec-"));
  const posted: string[] = [];
  let now = NOW_S * 1000;
  const d = { cfg: cfg(dir), service: { reserves: async (): Promise<ReserveReport[]> => { throw new Error("no RPC reports chain id 4663"); } }, caps: capsDown,
    channel: { send: async (t: string) => { posted.push(t); } }, store: new Store(dir), log: () => {}, nowMs: () => now, stackStage: 0 };
  for (let h = 0; h < 24 * 7; h++) { await runOnce(d); now += 3_600_000; }
  assert.match(posted[0], /SEV2 · reserves\nreserves UNVERIFIED \(the reconciler\) — the reconciler CANNOT READ: no collection could be read/);
  assert.equal(posted.length, 28, "at once, then every 6 hours for the week (168 / 6)");
});

test("E-08a: a partial unverified (one provider's bad hour) is quiet for 6 h, then paged; recovery is said once", async () => {
  const dir = mkdtempSync(join(tmpdir(), "w2-rec2-"));
  const posted: string[] = [];
  let now = NOW_S * 1000, reports = [APES];
  const caps = async () => compareCaps({ collections: [], global_max_items: 6, global_max_value_wei: 10n * E }, [], 0, now / 1000);
  const d = { cfg: cfg(dir), service: { reserves: async () => reports }, caps, channel: { send: async (t: string) => { posted.push(t); } }, store: new Store(dir), log: () => {}, nowMs: () => now, stackStage: 0 };
  for (let h = 0; h < 6; h++) { await runOnce(d); now += 3_600_000; }
  assert.equal(posted.length, 0, "under 6 h: shown, not paged");
  await runOnce(d);
  assert.match(posted[0], /reserves UNVERIFIED \(the reconciler\) — for 6 h/);
  reports = [FROGS]; now += 3_600_000;
  await runOnce(d);
  assert.match(posted.at(-1)!, /INFO · reserves\nreserves back to/);
});

test("E-08b regression: no Telegram token (or chat) is a startup refusal of the reconciler, never a red 'sent' to the log", () => {
  assert.throws(() => new Telegram({ token: undefined, chat: "123" }), /refusing to start: ALERT_TELEGRAM_BOT_TOKEN is not set/);
  assert.throws(() => new Telegram({ token: "x", chat: undefined }), /ALERT_TELEGRAM_CHAT_ID is not set/);
});

test("a hung run is abandoned, and the watchdog (its own timer) still turns the report red and alerts it", async () => {
  await assert.rejects(bounded(new Promise(() => {}), 20), /the run did not finish in 0 s/);
  const dir = mkdtempSync(join(tmpdir(), "w2-rec3-"));
  const store = new Store(dir), posted: string[] = [];
  const caps = async () => compareCaps({ collections: [], global_max_items: 6, global_max_value_wei: 10n * E }, [], 0, NOW_S);
  await runOnce({ cfg: cfg(dir), service: { reserves: async () => [FROGS] }, caps, channel: { send: async (t: string) => { posted.push(t); } }, store, log: () => {}, nowMs: () => NOW_S * 1000, stackStage: 0 });
  const s = await alert({ cfg: cfg(dir), channel: { send: async (t: string) => { posted.push(t); } }, store, log: () => {} }, (NOW_S + 10_801) * 1000);
  assert.equal(s.servedStatus, "red");
  assert.match(posted.at(-1)!, /reserves RED/);
  assert.equal(served(store.load().last, NOW_S + 10_801, 10_800).stale, true);
});

test("--test-alert goes through the reconciler's OWN channel: a red, an unverified and a recovery, each marked TEST", async () => {
  const posted: string[] = [];
  const n = await testAlerts({ send: async (t: string) => { posted.push(t); } } as Channel, "https://reserves.example");
  assert.equal(n, 3);
  assert.ok(posted.every((t) => t.startsWith("🧪 TEST — no action needed.")));
  assert.match(posted[0], /reserves RED \(https:\/\/reserves\.example\)/);
  assert.match(posted[1], /CANNOT READ/);
  assert.match(posted[2], /back to green/);
});
