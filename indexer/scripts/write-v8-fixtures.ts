import { mkdirSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { servedStatus } from "../src/v8/ethReserve.js";
import { checkReserves } from "../src/v8/selfcheck.js";
import { V8Service } from "../src/v8/service.js";
import type { SourceKind } from "../src/v8/types.js";
import { buildV8World, ETH_RPCS, GEN, HOME_RPCS, RELAYS, V8ADDR, v8Fetch } from "../test/v8World.js";

/**
 * docs/v8: writes indexer/fixtures/v8/ — the REAL v8 service and the REAL self-check run over test/v8World.ts's in-memory Milady vault
 * (fake Robinhood Chain and Ethereum, REAL drand evmnet beacons), so the draw proofs in it verify in the browser with no network.
 *   npm run fixtures:v8        then copy the folder to web-rook/public/fixtures/v8 (web-rook/test/copies.test.ts checks the copy)
 * Reserve reports are moved forward in time by the site (they must be fresh to be green); draw proofs are not (their times are
 * pinned to the real beacons' rounds).
 */
const NOTE = "FIXTURE. Nothing here is deployed. A Milady vault of 20 items and 9 tickets, drawn with REAL drand evmnet beacons (rounds served by api.drand.sh and api2.drand.sh on 2026-09-27) "
  + "through EthNftWrapper's own formula, over an invented Robinhood Chain and Ethereum. The collection's address is Milady's; the wrapper, custodian, module, transactions and every trait are invented, "
  + "and the explorer links point at nothing. Ticket 4's burn is made to look back-dated and ticket 5's batch unreadable, to show the verifier's timing flag.";
const source: SourceKind = { kind: "fixture", note: NOTE };
const w = buildV8World(), now = GEN + 30;
const collection = { slug: "milady", name: "Milady Maker", symbol: "wMIL", wrapper: V8ADDR.wrapper, collection: V8ADDR.collection, custodian: V8ADDR.custodian, module: V8ADDR.module, fromBlock: 65_000_000,
  snapshot: { file: "", url: "/v1/v8/snapshots/milady", pinnedIn: "deployments/launch/nft-pilot.json (fixture: not pinned anywhere)" } };
const svc = new V8Service({ config: { collections: [collection], homeRpcs: HOME_RPCS, ethRpcs: ETH_RPCS, relays: RELAYS, logChunk: 5_000_000 }, fetch: v8Fetch(w, { now }), now: () => now,
  source, canisterRecords: async () => w.canister, snapshots: new Map([["milady", w.snapshot]]) });

const reserve = (await svc.reserve("milady"))!;
const draws = (await svc.draws("milady"))!;
const snap = svc.snapshot("milady")!;
const self = await checkReserves({ slug: "milady", name: "Milady Maker", symbol: "wMIL", wrapper: V8ADDR.wrapper, collection: V8ADDR.collection, custodian: V8ADDR.custodian, module: V8ADDR.module,
  homeRpcs: HOME_RPCS, ethRpcs: ETH_RPCS, fetch: v8Fetch(w, { now }), now: () => now });
if (reserve.status !== "green" || self.report.status !== "green") throw new Error(`the fixture world is not green: ${reserve.reasons.join("; ")} ${self.report.reasons.join("; ")}`);

const dir = new URL("../fixtures/v8/", import.meta.url).pathname;
mkdirSync(dir, { recursive: true });
for (const f of readdirSync(dir)) rmSync(`${dir}${f}`);
const files: Record<string, string> = {};
const put = (path: string, file: string, body: unknown) => { files[path] = file; writeFileSync(`${dir}${file}`, JSON.stringify(body, null, 1) + "\n"); };
put("/v1/v8", "v8.json", { collections: svc.slugs(), endpoints: ["/v1/v8", "/v1/v8/reserves/:slug", "/v1/v8/draws/:slug", "/v1/v8/snapshots/:slug"] });
put("/v1/v8/reserves/milady", "reserves-milady.json", { ...reserve, served: servedStatus(reserve, now) });
put("/v1/v8/draws/milady", "draws-milady.json", draws);
put("/v1/v8/snapshots/milady", "snapshot-milady.json", snap.snapshot);
// not an endpoint: a recording of the browser's "check it yourself" run over the same world, shown in fixture mode instead of running it
put("selfcheck/milady", "selfcheck-milady.json", { source, hosts: { eth: ETH_RPCS, home: HOME_RPCS }, report: self.report, calls: self.calls });
writeFileSync(`${dir}index.json`, JSON.stringify({ kind: "v8", note: NOTE, generatedAt: now, files }, null, 1) + "\n");
console.log(`wrote ${Object.keys(files).length + 1} files to ${dir}: reserves ${reserve.status}, ${draws.proofs.length} proofs, snapshot ${snap.ref.sha256.slice(0, 18)}…, ${self.calls.length} self-check calls`);
