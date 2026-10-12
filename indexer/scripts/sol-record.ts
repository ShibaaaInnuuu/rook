// Fixture recorder (read-only): scans recent DBC transactions and saves one of each interesting kind to test/fixtures/sol/.
//   node --import tsx scripts/sol-record.ts [scan=150]
import { writeFileSync, existsSync } from "node:fs";
import { solRpc } from "../src/sol/rpc.js";
import { decodeTx, type ParsedTx } from "../src/sol/decode.js";
import { DBC_PROGRAM, type DbcSwapEvent } from "../src/sol/dbc.js";
const scan = Number(process.argv[2] ?? 150);
const rpc = solRpc([process.env.SOLANA_RPC_URL ?? "https://api.mainnet-beta.solana.com", "https://solana-rpc.publicnode.com"], { minGapMs: 250 });
const want = new Map<string, string>([["exact-out", ""], ["cpi-swap", ""], ["mode0-buy", ""], ["v1tx", ""]]);
let before: string | undefined;
const configs = new Map<string, number>();
for (let seen = 0; seen < scan;) {
  const sigs = await rpc.call<{ signature: string; err: unknown }[]>("getSignaturesForAddress", [DBC_PROGRAM, { limit: 50, ...(before ? { before } : {}) }]);
  if (!sigs.length) break;
  before = sigs[sigs.length - 1].signature;
  for (const s of sigs) {
    if (s.err) continue;
    seen++;
    const tx = await rpc.call<ParsedTx & { version?: number | string }>("getTransaction", [s.signature, { encoding: "jsonParsed", maxSupportedTransactionVersion: 1, commitment: "confirmed" }]);
    if (!tx) continue;
    const d = decodeTx(tx);
    const outerDbc = tx.transaction.message.instructions.some((i) => i.programId === DBC_PROGRAM);
    for (const sw of d.swaps) {
      if (sw.ev.kind !== "dbc-swap") continue;
      const ev = sw.ev as DbcSwapEvent;
      configs.set(ev.config, (configs.get(ev.config) ?? 0) + 1);
      const tag = ev.swapMode === 2 ? "exact-out" : !outerDbc ? "cpi-swap" : ev.direction === 1 && ev.inputExcludedFee !== null && ev.inputExcludedFee !== ev.inputAmount ? "mode0-buy" : String(tx.version) === "1" ? "v1tx" : null;
      if (tag && want.get(tag) === "") { want.set(tag, s.signature); const f = `test/fixtures/sol/${tag}-${s.signature.slice(0, 10)}.json`; if (!existsSync(f)) writeFileSync(f, JSON.stringify(tx)); console.log("saved", tag, s.signature); }
    }
  }
  if ([...want.values()].every(Boolean)) break;
}
console.log(JSON.stringify(Object.fromEntries(want)), "busiest configs:", [...configs.entries()].sort((a, b) => b[1] - a[1]).slice(0, 5));
