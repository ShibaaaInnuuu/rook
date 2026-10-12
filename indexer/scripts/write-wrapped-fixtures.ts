import { mkdirSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import type { WrappedMarket } from "../types.js";
import { parseConfig } from "../src/wrapped/config.js";
import { WrappedService } from "../src/wrapped/service.js";
import { buildWorld, FROGS, APES, NOW, worldConfig } from "../test/wrappedWorld.js";

/**
 * docs/v5/ux-and-reserves.md §3. Writes indexer/fixtures/wrapped/: the REAL reserve service run over an in-memory world built from
 * docs/v3/curve.md's worked examples. Nothing in it is deployed; every file says `source.kind: "fixture"`, and the site shows it only
 * under ?demo=sample with a FIXTURE label. One of the apes' two EVM providers is made to fail, so the set also shows what an
 * `unverified` collection looks like.
 *   npm run fixtures:wrapped            then copy the folder to web/public/fixtures/wrapped (web/test/api.test.ts checks the copy)
 */
const E = 10n ** 18n;
const NOTE = "FIXTURE. Nothing on this page is deployed. The collections, coins, items, tickets and checks are built from the worked examples in docs/v3/curve.md "
  + "(frogs at 5 ETH; an ape bought FIFO for two coins), with a floor module, a treasury, a YOYO coin and an offer board given plausible books, and run through the real reserve computation. Addresses, inscription ids and transactions are invented; "
  + "the explorer links point at nothing. One of the apes' two EVM providers is made to fail on purpose, to show an unverified collection.";
const market: Record<string, WrappedMarket> = {
  frogs: {
    // shared wFROG/ETH pool: a full-range estimate at the keeper's 0.646 ETH cap (docs/v3/curve.md's depth after ~5 graduations)
    shared: { reserveEth: String(33n * E), reserveWrapped: String(51n * E), lpFeePips: 3_000, protocolFeePips: 500 },
    coins: {
      [FROGS.alien]: { curve: null, pool: { reserveWrapped: "5453000000000000000", reserveCoin: String(200_000_000n * E), lpFeePips: 10_000, protocolFeePips: 1_000 } },
      // BETA: the 5 ETH curve of the worked example, 1.2 ETH in (v0 = 2.790 ETH, T0 = 1.246e9, ends at 7.790 ETH / 4.464e8)
      [FROGS.beta]: { curve: { v: "3990000000000000000", t: "871263157894736842105263158", vEnd: "7790000000000000000", tEnd: String(446_400_000n * E), feeBps: 300 }, pool: null },
      // GAMMA: a YOYO coin at the 5% tier, graduated; its pool charges the tier as its LP fee (docs/v5/policies-and-fees.md §3)
      [FROGS.gamma]: { curve: null, pool: { reserveWrapped: "5453000000000000000", reserveCoin: String(200_000_000n * E), lpFeePips: 50_000, protocolFeePips: 5_000 } },
    },
    // the module as the quote engine needs it (docs/v5/floor-module.md §13.4, §6.1): the fills price, each side's spread, the room
    floorModule: { priceWei: "640000000000000000", spreadMintBps: 21, spreadBurnBps: 15, mintRoomWei: String(3n * E), burnRoomUnits: "625000000000000000" },
  },
  apes: {
    shared: { reserveEth: String(20n * E), reserveWrapped: "3921568627450980392", lpFeePips: 3_000, protocolFeePips: 500 },
    coins: { [APES.coinA]: { curve: null, pool: { reserveWrapped: "684000000000000000", reserveCoin: String(200_000_000n * E), lpFeePips: 10_000, protocolFeePips: 1_000 } } },
    floorModule: null,
  },
};

const w = buildWorld();
w.failing.add("http:fixture-provider-b");
const svc = new WrappedService({ config: parseConfig({ ...worldConfig(), source: { kind: "fixture", note: NOTE }, market }), rpc: w.rpc, fetch: w.fetch, now: () => NOW });

const dir = new URL("../fixtures/wrapped/", import.meta.url).pathname;
mkdirSync(dir, { recursive: true });
for (const f of readdirSync(dir)) rmSync(dir + f);
const files: Record<string, string> = {};
const put = (path: string, file: string, body: unknown) => { files[path] = file; writeFileSync(dir + file, JSON.stringify(body, null, 2) + "\n"); };
put("/v1/wrapped", "wrapped.json", await svc.list());
put("/v1/reserves", "reserves.json", { configured: true, reports: await svc.reserves() });
for (const slug of ["frogs", "apes"]) {
  const c = (await svc.collection(slug))!;
  put(`/v1/wrapped/${slug}`, `wrapped-${slug}.json`, c);
  put(`/v1/reserves/${slug}`, `reserves-${slug}.json`, await svc.reserve(slug));
  for (let id = 0; ; id++) { const t = await svc.ticket(slug, id); if (!t) break; put(`/v1/wrapped/${slug}/tickets/${id}`, `ticket-${slug}-${id}.json`, t); }
  put(`/v1/handins/${slug}`, `handins-${slug}.json`, await svc.handins(slug));
}
writeFileSync(dir + "index.json", JSON.stringify({ kind: "wrapped", note: NOTE, generatedAt: NOW, files }, null, 2) + "\n");
console.log(`wrote ${Object.keys(files).length} answers to ${dir}`);
