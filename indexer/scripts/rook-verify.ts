import { readFileSync } from "node:fs";
import { verifyDraw } from "../src/v8/draw.js";
import { DRAND_RELAYS } from "../src/v8/drand.js";
import { servedStatus } from "../src/v8/ethReserve.js";
import { fmtUnits } from "../src/v8/ethReserve.js";
import { canonicalJson, snapshotHash } from "../src/v8/rarity.js";
import { checkDraw, checkReserves, DEFAULT_ETH_RPCS, DEFAULT_HOME_RPCS } from "../src/v8/selfcheck.js";
import type { DrawProof, DrawReport, MetadataSnapshot } from "../src/v8/types.js";

/**
 * docs/v8/proof-of-reserves.md §4.2 and redemption-draw.md §2.3: the keyless command-line verifier. It needs no Rook service: every
 * number is read from the providers named on the command line (default: the keyless pair independent of the canister's and the
 * monitor's), and every draw is recomputed from drand's relays and Robinhood Chain. The same modules the page runs (src/v8/).
 *   npm run -s verify -- reserves --wrapper 0x… --collection 0x… --custodian 0x… [--module 0x…] [--eth-rpc URL]… [--rh-rpc URL]… [--json]
 *   npm run -s verify -- draw --proof proof.json|URL [--wrapper 0x…] [--rh-rpc URL] [--eth-rpc URL] [--relay URL]… [--offline]
 *   npm run -s verify -- draws --report draws.json|URL [--offline]           every proof in a /v1/v8/draws report
 *   npm run -s verify -- snapshot --file snapshot.json|URL --sha256 0x…       the metadata snapshot against its pinned hash
 * Exit: 0 green / verified; 1 red / mismatch; 2 unverified / incomplete; 64 a usage error. Decision P3: packaged for npm as
 * @rook/verify by scripts/bundle-verify.mjs (not published: publishing is the operator's).
 */
const argv = process.argv.slice(2), cmd = argv[0];
const all = (k: string) => argv.flatMap((a, i) => (a === `--${k}` && argv[i + 1] ? [argv[i + 1]] : []));
const one = (k: string) => all(k)[0];
const flag = (k: string) => argv.includes(`--${k}`);
const usage = (why: string): never => { console.error(`${why}\n\n${readFileSync(new URL(import.meta.url), "utf8").split("\n").filter((l) => l.startsWith(" *   ")).join("\n")}`); process.exit(64); };
const addr = (k: string): `0x${string}` => { const v = one(k); if (!v || !/^0x[0-9a-fA-F]{40}$/.test(v)) usage(`--${k} must be an address`); return v!.toLowerCase() as `0x${string}`; };
const maybeAddr = (k: string) => (one(k) ? addr(k) : null);
async function load<T>(where: string): Promise<T> {
  if (/^https?:\/\//.test(where)) {
    const r = await fetch(where, { signal: AbortSignal.timeout(15_000), headers: { accept: "application/json" } });
    if (!r.ok) throw new Error(`${where}: HTTP ${r.status}`);
    const t = await r.text(); if (t.length > 16_000_000) throw new Error(`${where}: too large`);
    return JSON.parse(t) as T;
  }
  return JSON.parse(readFileSync(where, "utf8")) as T;
}
const mark = (ok: boolean | null) => (ok === true ? "ok  " : ok === false ? "FAIL" : " -- ");

async function reserves() {
  const eth = all("eth-rpc").length ? all("eth-rpc") : [...DEFAULT_ETH_RPCS], home = all("rh-rpc").length ? all("rh-rpc") : [...DEFAULT_HOME_RPCS];
  const { report, calls } = await checkReserves({ slug: one("slug") ?? "collection", name: one("name") ?? "collection", symbol: "", wrapper: addr("wrapper"), collection: addr("collection"),
    custodian: addr("custodian"), module: maybeAddr("module"), homeRpcs: home, ethRpcs: eth, fetch, arrivals: !flag("no-arrivals") });
  const served = servedStatus(report, Math.floor(Date.now() / 1000));
  if (flag("json")) console.log(JSON.stringify({ report, served, calls }, null, 1));
  else {
    console.log(`${served.status.toUpperCase()}  ${report.equation.inCustody} items in custody − ${report.equation.ticketsInFlight} in flight + ${fmtUnits(BigInt(report.equation.ethBackedUnits))} ETH-backed = ${fmtUnits(BigInt(report.equation.backingUnits))} vs supply ${report.equation.supplyUnits === null ? "?" : fmtUnits(BigInt(report.equation.supplyUnits))}`);
    console.log(`Robinhood Chain block ${report.homeBlock ?? "?"} (${home.map((u) => new URL(u).host).join(", ")}); Ethereum finalized ${report.ethBlock ?? "?"} (${eth.map((u) => new URL(u).host).join(", ")})`);
    for (const r of served.reasons) console.log(`  - ${r}`);
    for (const i of report.items) console.log(`  #${i.tokenId.padEnd(6)} ${i.verdict.padEnd(11)} ${i.owners.map((o) => `${o.source}=${o.owner ?? (o.reverted ? "revert" : "no answer")}`).join(" ")}  arrival: ${i.arrival.verdict}`);
    console.log(`${calls.length} calls, ${calls.filter((c) => !c.ok).length} failed (--json lists them)`);
  }
  process.exit(served.status === "green" ? 0 : served.status === "red" ? 1 : 2);
}

async function draws(list: DrawProof[], wrapper: `0x${string}`) {
  let worst = 0;
  for (const p of list) {
    const r = flag("offline") ? { verdict: verifyDraw(p, { wrapper }), calls: [], notes: ["offline: the proof is checked against itself and evmnet's public key only"] }
      : await checkDraw({ wrapper, proof: p, homeRpcs: all("rh-rpc").length ? all("rh-rpc") : [DEFAULT_HOME_RPCS[0]], ethRpcs: all("eth-rpc").length ? all("eth-rpc") : [DEFAULT_ETH_RPCS[0]],
        relays: all("relay").length ? all("relay") : DRAND_RELAYS.slice(0, 2), fetch });
    console.log(`ticket ${p.ticket}: ${r.verdict.verdict.toUpperCase()}  (round ${p.round}, item ${p.itemId}, #${p.tokenId})`);
    for (const c of r.verdict.checks) console.log(`  ${mark(c.ok)} ${c.label}: ${c.detail}`);
    console.log(`  timing: ${r.verdict.timing.level} — ${r.verdict.timing.text}`);
    for (const n of r.notes) console.log(`  note: ${n}`);
    worst = Math.max(worst, r.verdict.verdict === "mismatch" ? 1 : r.verdict.verdict === "incomplete" ? 2 : 0);
  }
  process.exit(worst === 1 ? 1 : worst === 2 ? 2 : 0);
}

async function main() {
  if (cmd === "reserves") return reserves();
  if (cmd === "draw") { const p = await load<DrawProof & { wrapper?: string }>(one("proof") ?? usage("--proof is required")); return draws([p], (one("wrapper") ?? p.wrapper ?? usage("--wrapper is required")) as `0x${string}`); }
  if (cmd === "draws") { const r = await load<DrawReport>(one("report") ?? usage("--report is required")); return draws(r.proofs, r.wrapper); }
  if (cmd === "snapshot") {
    const s = await load<MetadataSnapshot>(one("file") ?? usage("--file is required")), want = (one("sha256") ?? usage("--sha256 is required")).toLowerCase();
    const got = snapshotHash(s);
    console.log(`${got === want ? "OK  " : "FAIL"} sha256(canonical JSON) = ${got} (${canonicalJson(s).length} bytes, ${s.tokens.length} tokens)`);
    process.exit(got === want ? 0 : 1);
  }
  usage(cmd ? `unknown command ${cmd}` : "a command is required");
}
main().catch((e) => { console.error(String((e as Error)?.message ?? e)); process.exit(2); });
