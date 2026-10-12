// Exploration helper (read-only): fetch recent transactions of an address and print what decodeTx makes of them.
//   node --import tsx scripts/sol-explore.ts <address> [limit] [outDir]
import { writeFileSync } from "node:fs";
import { solRpc } from "../src/sol/rpc.js";
import { decodeTx, type ParsedTx } from "../src/sol/decode.js";
const [addr, lim = "20", outDir] = process.argv.slice(2);
const rpc = solRpc([process.env.SOLANA_RPC_URL ?? "https://api.mainnet-beta.solana.com", "https://solana-rpc.publicnode.com"], { minGapMs: 150 });
const sigs = await rpc.call<{ signature: string; err: unknown }[]>("getSignaturesForAddress", [addr, { limit: Number(lim) }]);
const j = (x: unknown) => JSON.stringify(x, (_k, v) => (typeof v === "bigint" ? v.toString() : v));
for (const s of sigs) {
  if (s.err) continue;
  const tx = await rpc.call<ParsedTx>("getTransaction", [s.signature, { encoding: "jsonParsed", maxSupportedTransactionVersion: 1, commitment: "confirmed" }]);
  if (!tx) continue;
  const d = decodeTx(tx);
  const kinds = [...d.swaps.map((x) => x.ev.kind), ...d.inits.map(() => "init"), ...d.completes.map(() => "complete"), ...d.migrations.map((m) => "migrate-" + m.version)];
  console.log(s.signature.slice(0, 12), kinds.join(","));
  for (const sw of d.swaps) {
    const pre = tx.meta!.preBalances, post = tx.meta!.postBalances, keys = tx.transaction.message.accountKeys.map((k) => (typeof k === "string" ? k : k.pubkey));
    const ti = keys.indexOf(sw.trader);
    console.log("   ", j({ ...sw.ev, trader: sw.trader, solDelta: ti >= 0 ? post[ti] - pre[ti] : null, fee: tx.meta!.fee }));
  }
  for (const i of d.inits) console.log("   init", j(i));
  for (const m of d.migrations) console.log("   mig", j(m));
  if (outDir && kinds.length) writeFileSync(`${outDir}/${kinds[0]}-${s.signature.slice(0, 10)}.json`, JSON.stringify(tx));
}
