// The runner's UniSat step, against a fake UniSat and a fake canister: it picks a member listing under the cap, asks UniSat for the
// purchase PSBT built for the float and custody, and hands it to the canister's classic judge. No network.
import assert from "node:assert/strict";
import { test } from "node:test";
import { logger } from "../src/log.js";
import { psbtHexOf, UNISAT_BID_EVERY_S, unisatOnce, type UnisatBuyer, type UnisatDeps } from "../src/unisatbuy.js";

const SATS_PER_ETH = 3_000_000n;                       // 0.03 BTC per ETH
const CAP_WEI = 150_000_000_000_000_000n;              // 0.15 ETH = 450,000 sats
const MEMBERS = ["aai0", "bbi0", "cci0", "ddi0"];
const row = (id: string, price: number, marketType = "fixedPrice") => ({ auctionId: `auc-${id}`, inscriptionId: id, marketType, price, unitPrice: price, satoshi: 546, address: "bc1pseller" });

function world(o: { rows?: unknown[]; phase?: Record<string, unknown>; refuse?: string; bid?: Record<string, unknown>; code?: number; key?: string | undefined } = {}) {
  const calls: { path: string; body: Record<string, unknown>; auth: string | null }[] = [], handed: { coll: number; id: string; psbt: string; toSign: number[]; proof: string[] }[] = [], asked: ([] | [number])[] = [];
  const rows = o.rows ?? [row("aai0", 500_000), row("bbi0", 420_000), row("cci0", 400_000, "auction"), row("zzi0", 100_000), row("ddi0", 430_000)];
  const logs: string[] = [];
  const fetchFake = (async (url: string, init?: RequestInit) => {
    const path = String(url).replace("https://open-api.unisat.io", "");
    calls.push({ path, body: JSON.parse(String(init?.body)), auth: (init?.headers as Record<string, string>)?.authorization ?? null });
    const json = (v: unknown) => ({ ok: true, status: 200, text: async () => JSON.stringify(v) });
    if (o.code !== undefined) return json({ code: o.code, msg: "nope" });
    if (path === "/v3/market/collection/auction/list") return json({ code: 0, data: { list: rows, total: rows.length } });
    if (path === "/v3/market/collection/auction/create_bid") return json({ code: 0, data: o.bid ?? { bidId: "bid-1", psbtBid: Buffer.from("70736274ff0102", "hex").toString("base64"), bidSignIndexes: [1, 2] } });
    return json({ code: -1, msg: "unexpected" });
  }) as unknown as typeof fetch;
  const buyer: UnisatBuyer = {
    frog_buys: async (c) => { asked.push(c); return [{ id: 7n, phase: o.phase ?? { Wanted: { cap_wei: CAP_WEI } } }]; },
    float_address: async () => ({ Ok: "bc1pfloat 11" }), float_internal_key: async () => ({ Ok: "22".repeat(32) }), custody_address: async () => ({ Ok: "bc1pcustody 33" }),
    frog_offer: async (coll, _id, inscription, psbt, toSign, proof) => { if (o.refuse) return { Err: o.refuse }; handed.push({ coll, id: inscription, psbt, toSign, proof }); return { Ok: "stored" }; },
  };
  let now = 1_000_000;
  const d: UnisatDeps = { buyer, fetch: fetchFake, log: logger((l) => logs.push(l)), collectionId: "bitcoin-frogs", members: MEMBERS, coll: 3,
    apiKey: () => ("key" in o ? o.key : "secret-key"), satsPerEth: async () => SATS_PER_ETH, feeRate: async () => 4, nowS: () => now };
  return { d, calls, handed, logs, asked, tick: (s: number) => { now += s; } };
}

test("hands the cheapest fixed-price MEMBER listing under the cap to the classic judge, built for the float and delivered to custody", async () => {
  const { d, calls, handed, asked } = world();
  assert.equal(await unisatOnce(d), true);
  assert.equal(handed.length, 1);
  assert.deepEqual([asked[0], handed[0].coll], [[3], 3], "Ordinals multi-collection: the frog buys read and the offer handed in are the collection's (coll 3)");
  assert.equal(handed[0].id, "bbi0", "420,000 sats: cci0 is an auction, zzi0 not a member, aai0 over the 450,000 cap");
  assert.equal(handed[0].psbt, "70736274ff0102", "base64 turned into hex");
  assert.deepEqual(handed[0].toSign, [1, 2]);
  assert.ok(handed[0].proof.length > 0 && handed[0].proof.every((h) => /^[0-9a-f]{64}$/.test(h)));
  const bid = calls.find((c) => c.path.endsWith("/create_bid"))!;
  assert.deepEqual(bid.body, { auctionId: "auc-bbi0", bidPrice: 420_000, address: "bc1pfloat", pubkey: "22".repeat(32), feeRate: 4, nftAddress: "bc1pcustody" });
  assert.equal(bid.auth, "Bearer secret-key", "the key goes only in UniSat's header");
  assert.deepEqual((calls[0].body as { filter: unknown }).filter, { nftType: "collection", collectionId: "bitcoin-frogs", isEnd: false });
});

test("one bid per frog per UNISAT_BID_EVERY_S, whatever the outcome", async () => {
  const { d, calls, handed, tick } = world({ refuse: "SellerNotSigned" });
  assert.equal(await unisatOnce(d), false);
  assert.equal(await unisatOnce(d), false);
  assert.equal(calls.filter((c) => c.path.endsWith("/create_bid")).length, 1);
  tick(UNISAT_BID_EVERY_S);
  await unisatOnce(d);
  assert.equal(calls.filter((c) => c.path.endsWith("/create_bid")).length, 2);
  assert.equal(handed.length, 0);
});

test("a refusal by the canister (e.g. the seller's input unsigned) is logged, nothing else happens", async () => {
  const { d, logs } = world({ refuse: "the seller's input is not signed" });
  assert.equal(await unisatOnce(d), false);
  assert.ok(logs.some((l) => l.includes("unisat-hand-in-refused")));
});

test("nothing wanted, nothing under the cap, a bid without inputs to sign, or a UniSat error: nothing handed in", async () => {
  for (const o of [{ phase: { Buying: {} } }, { rows: [row("aai0", 900_000)] }, { bid: { bidId: "b", psbtBid: "70736274ff00", bidSignIndexes: [] } },
    { bid: { bidId: "b", psbtBid: "not a psbt", bidSignIndexes: [0] } }]) {
    const { d, handed } = world(o);
    assert.equal(await unisatOnce(d), false);
    assert.equal(handed.length, 0);
  }
  await assert.rejects(unisatOnce(world({ code: 10001 }).d), /code 10001/);
  await assert.rejects(unisatOnce(world({ key: undefined }).d), /UNISAT_API_KEY/);
});

test("psbtHexOf takes hex or base64 and nothing else", () => {
  assert.equal(psbtHexOf("70736274FF00"), "70736274ff00");
  assert.equal(psbtHexOf(Buffer.from("70736274ff00", "hex").toString("base64")), "70736274ff00");
  assert.throws(() => psbtHexOf("deadbeef"));
});
