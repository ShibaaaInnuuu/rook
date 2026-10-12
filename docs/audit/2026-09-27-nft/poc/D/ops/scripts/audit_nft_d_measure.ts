// Audit D (2026-09-27-nft), read-only live measurement: what the HONEST runner hands in as "sales" for each launch collection.
// It calls the runner's own recentTransferTxs(…, 12) (ops/src/ethnft.ts) against a public Ethereum RPC, then reads each receipt
// and counts the sales the canister's sales_in would keep (a Seaport 1.5/1.6 OrderFulfilled or Blur v2 Execution721*Packed that
// names the collection). Usage: npx tsx scripts/audit_nft_d_measure.ts [rpc]
import { recentTransferTxs } from "../src/ethnft.js";
const RPC = process.argv[2] ?? "https://mainnet.gateway.tenderly.co";
const C: Record<string, string> = { milady: "0x5af0d9827e0c53e4799bb226655a1de152a425a5", azuki: "0xed5af388653567af2f388e6224dc7c4b3241c544", mayc: "0x60e4d786628fea6478f785a6d7e704777c86a7c6" };
const SEAPORT = ["0x0000000000000068f116a894984e2db1123eb395", "0x00000000000000adc04c56bf30ac9d3c0aaf14dc"], OF = "0x9d9af8e38d66c62e2c12f0225249fd9d721c54b83f48d9352c97c6cacdcb6f31";
const BLUR = "0xb2ecfe4e4d61f8790bbb9de2d1259b9e2410cea5", BT = ["0x1d5e12b51dee5e4d34434576c3fb99714a85f57b0fd546ada4b0bddd736d12b2", "0x0fcf17fac114131b10f37b183c6a60f905911e52802caeeb3e6ea210398b81ab", "0x7dc5c0699ac8dd5250cbe368a2fc3b4a2daadb120ad07f6cccea29f83482686e"];
const log = () => {};
async function rpc(method: string, params: unknown[]) { const r = await fetch(RPC, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) }); const j = await r.json() as any; if (j.error) throw new Error(JSON.stringify(j.error)); return j.result; }
for (const [name, c] of Object.entries(C)) {
  const d = { ethRpc: RPC, fetch, log } as any;
  const txs = await recentTransferTxs(d, c, 12);
  let sales = 0, saleTxs = 0;
  for (const h of txs) {
    const r = await rpc("eth_getTransactionReceipt", [h]);
    const n = (r.logs as any[]).filter((l) => ((SEAPORT.includes(l.address.toLowerCase()) && l.topics[0] === OF) || (l.address.toLowerCase() === BLUR && BT.includes(l.topics[0])))
      && l.data.toLowerCase().includes(c.slice(2))).length;
    sales += n; if (n) saleTxs++;
  }
  console.log(JSON.stringify({ at: new Date().toISOString(), collection: name, handedIn: txs.length, txsWithASale: saleTxs, saleLogs: sales, minSalesMet: saleTxs >= 3 }));
}
