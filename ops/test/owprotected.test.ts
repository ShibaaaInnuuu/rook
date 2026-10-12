// docs/v8/ow-protected.md §4: the runner's half of an Ordinals Wallet protected purchase, against a fake Ordinals Wallet and a fake
// canister. It picks and carries; the canister judges and signs. No network.
import assert from "node:assert/strict";
import { test } from "node:test";
import { logger } from "../src/log.js";
import { OW_COSIGNER, owProtectedOnce, type OwBuyer, type OwDeps, type ProtectedListingV } from "../src/owprotected.js";

const SATS_PER_ETH = 3_000_000n;                       // 0.03 BTC per ETH
const CAP_WEI = 150_000_000_000_000_000n;              // 0.15 ETH = 450,000 sats
const MEMBERS = ["aai0", "bbi0", "cci0", "ddi0"];
const caps = { secure_purchase: { mode: "passthrough_v4", cosigner_public_key: OW_COSIGNER, build_enabled: true, submit_enabled: true, protocols: ["ordinal"] } };
const row = (id: string, price: number, prot = true) => ({ inscription_id: id, outpoint: `${id.slice(0, 2).repeat(32)}:0`, satoshi_price: price, protected: prot, seller_address: "bc1qseller" });

function world(o: { rows?: unknown[]; caps?: unknown; phase?: Record<string, unknown>; signed?: boolean; refuse?: string } = {}) {
  const calls: { path: string; body?: unknown }[] = [], handed: { coll: number; id: string; listing: ProtectedListingV; setup: [] | [string]; proof: string[] }[] = [], asked: ([] | [number])[] = [];
  const rows = o.rows ?? [row("aai0", 500_000), row("bbi0", 420_000), row("cci0", 400_000, false), row("zzi0", 100_000), row("ddi0", 430_000)];
  const fetchFake = (async (url: string, init?: RequestInit) => {
    const path = String(url).replace("https://turbo.ordinalswallet.com", "");
    const body = init?.body ? JSON.parse(String(init.body)) : undefined;
    calls.push({ path, body });
    const json = (v: unknown, status = 200) => ({ ok: status === 200, status, text: async () => JSON.stringify(v) });
    if (path === "/market/secure-purchase/capabilities") return json(o.caps ?? caps);
    if (path === "/wallet/fee-estimates") return json({ halfHourFee: 3.2, fastestFee: 5 });
    if (path.startsWith("/collection/")) return json(rows);
    if (path.startsWith("/market/escrow/")) { const id = decodeURIComponent(path.split("/").pop()!); return json({ protected: true, secure_purchase_state: "listed", seller_address: "bc1qseller", satoshi_price: (rows as { inscription_id: string; satoshi_price: number }[]).find((r) => r.inscription_id === id)!.satoshi_price, escrow_price: 400_000, creator_address: null }); }
    if (path === "/wallet/secure-purchase/build") return json({ cosigner_public_key: OW_COSIGNER, sales: [{ sale_txid: "ab".repeat(32), psbt: "70736274ff", parent: { txid: "cd".repeat(32), raw: "0200", source_outpoint: body.outpoints[0] } }], setup: { txid: "ef".repeat(32), psbt: "70736274ffee" } });
    if (path === "/market/secure-purchase/submit") return json({ accepted: true, txid: "ab".repeat(32), state: "broadcast" });
    return json({ error: true, message: "unexpected" }, 404);
  }) as unknown as typeof fetch;
  const buyer: OwBuyer = {
    frog_buys: async (c) => { asked.push(c); return [{ id: 7n, phase: o.phase ?? { Wanted: { cap_wei: CAP_WEI } } }]; },
    float_address: async () => ({ Ok: "bc1pfloat 11" }), float_internal_key: async () => ({ Ok: "22".repeat(32) }), custody_address: async () => ({ Ok: "bc1pcustody 33" }),
    frog_offer_protected: async (coll, _id, inscription, _s, _p, listing, setup, proof) => { if (o.refuse) return { Err: o.refuse }; handed.push({ coll, id: inscription, listing, setup, proof }); return { Ok: "stored" }; },
    frog_protected_signed: async () => (o.signed ? [{ sale_txid: "ab".repeat(32), sale_psbt: "70736274ff01", setup_psbt: [] }] : []),
  };
  const d: OwDeps = { buyer, fetch: fetchFake, log: logger(() => {}), slug: "bitcoin-frogs", members: MEMBERS, coll: 4, satsPerEth: async () => SATS_PER_ETH };
  return { d, calls, handed, asked };
}

test("hands in the cheapest PROTECTED member listing under the cap, built for the float, delivered to custody", async () => {
  const { d, calls, handed, asked } = world();
  assert.equal(await owProtectedOnce(d), true);
  assert.equal(handed.length, 1);
  assert.deepEqual([asked[0], handed[0].coll], [[4], 4], "Ordinals multi-collection: the frog buys read and the offer handed in are the collection's (coll 4)");
  assert.equal(handed[0].id, "bbi0", "420,000 sats: under the 450,000 cap; cci0 is cheaper but unprotected, zzi0 is not a member, aai0 is over the cap");
  assert.deepEqual(handed[0].listing.escrow_price, [400_000n]);
  assert.deepEqual(handed[0].setup, ["70736274ffee"]);
  assert.ok(handed[0].proof.length > 0 && handed[0].proof.every((h) => /^[0-9a-f]{64}$/.test(h)), "the membership proof, without 0x");
  const build = calls.find((c) => c.path === "/wallet/secure-purchase/build")!.body as Record<string, unknown>;
  assert.deepEqual(build, { outpoints: ["bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb:0"], protocol: "ordinal", from: "bc1pfloat", public_key: "22".repeat(32), to: "bc1pcustody", fee_rate: 4 });
});

test("refuses to trade when Ordinals Wallet names another co-signer or another mode", async () => {
  for (const c of [{ ...caps.secure_purchase, cosigner_public_key: "00".repeat(32) }, { ...caps.secure_purchase, mode: "passthrough_v5" }, { ...caps.secure_purchase, submit_enabled: false }]) {
    const { d, handed } = world({ caps: { secure_purchase: c } });
    await assert.rejects(owProtectedOnce(d), /not as pinned/);
    assert.equal(handed.length, 0);
  }
});

test("nothing under the cap: nothing handed in; a refusal by the canister moves on to the next candidate", async () => {
  const none = world({ rows: [row("aai0", 900_000)] });
  assert.equal(await owProtectedOnce(none.d), false);
  assert.equal(none.handed.length, 0);
  const refused = world({ refuse: "OverCap" });
  assert.equal(await owProtectedOnce(refused.d), false);
  assert.equal(refused.calls.filter((c) => c.path === "/wallet/secure-purchase/build").length, 2, "bbi0 then ddi0, both refused");
});

test("a Handed frog's signed sale is submitted once; the step signs nothing itself", async () => {
  const { d, calls } = world({ phase: { Handed: {} }, signed: true });
  assert.equal(await owProtectedOnce(d), true);
  assert.equal(await owProtectedOnce(d), false, "not again in this process");
  const subs = calls.filter((c) => c.path === "/market/secure-purchase/submit");
  assert.equal(subs.length, 1);
  assert.deepEqual(subs[0].body, { sales: [{ sale_txid: "ab".repeat(32), psbt: "70736274ff01" }] });
});

test("guard: the step has no key and no signing call", async () => {
  const { readFileSync } = await import("node:fs");
  const src = readFileSync(new URL("../src/owprotected.ts", import.meta.url), "utf8");
  assert.ok(!/privateKey|signTransaction|sign\(|OPERATOR_IDENTITY_PEM|schnorr/i.test(src.replace(/^\s*(\/\/|\*).*$/gm, "")));
});
