// Audit D (2026-09-27-nft), read-only: over the last day of blocks, for every point in time, how many of the runner's
// "latest 12 transfer transactions" (ops/src/ethnft.ts recentTransferTxs) carry a sale the canister would count.
// Usage: npx tsx scripts/audit_nft_d_window.ts [rpc]
const RPC = process.argv[2] ?? "https://mainnet.gateway.tenderly.co";
const C: Record<string, string> = { milady: "0x5af0d9827e0c53e4799bb226655a1de152a425a5", azuki: "0xed5af388653567af2f388e6224dc7c4b3241c544", mayc: "0x60e4d786628fea6478f785a6d7e704777c86a7c6" };
const SEAPORT = ["0x0000000000000068f116a894984e2db1123eb395", "0x00000000000000adc04c56bf30ac9d3c0aaf14dc"], OF = "0x9d9af8e38d66c62e2c12f0225249fd9d721c54b83f48d9352c97c6cacdcb6f31";
const BLUR = "0xb2ecfe4e4d61f8790bbb9de2d1259b9e2410cea5", BT = ["0x1d5e12b51dee5e4d34434576c3fb99714a85f57b0fd546ada4b0bddd736d12b2", "0x0fcf17fac114131b10f37b183c6a60f905911e52802caeeb3e6ea210398b81ab", "0x7dc5c0699ac8dd5250cbe368a2fc3b4a2daadb120ad07f6cccea29f83482686e"];
const TOPIC = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
async function rpc(method: string, params: unknown[]) { for (let i = 0; i < 4; i++) { try { const r = await fetch(RPC, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) }); const j = await r.json() as any; if (j.error) throw new Error(JSON.stringify(j.error)); return j.result; } catch (e) { if (i === 3) throw e; await new Promise((o) => setTimeout(o, 500)); } } }
const head = Number(BigInt(await rpc("eth_blockNumber", [])));
for (const [name, c] of Object.entries(C)) {
  // two days of logs, so every point of the last day sees its full 7,200-block lookback
  const logs = [...await rpc("eth_getLogs", [{ address: c, topics: [TOPIC], fromBlock: `0x${(head - 14400).toString(16)}`, toBlock: `0x${(head - 7201).toString(16)}` }]) as any[],
    ...await rpc("eth_getLogs", [{ address: c, topics: [TOPIC], fromBlock: `0x${(head - 7200).toString(16)}`, toBlock: `0x${head.toString(16)}` }]) as any[]];
  const txs: string[] = [], blk: number[] = []; for (const l of logs) if (!txs.includes(l.transactionHash)) { txs.push(l.transactionHash); blk.push(Number(BigInt(l.blockNumber))); }   // oldest first
  const sale: boolean[] = [];
  for (const h of txs) { const r = await rpc("eth_getTransactionReceipt", [h]);
    sale.push(r.status === "0x1" && (r.logs as any[]).some((l) => ((SEAPORT.includes(l.address.toLowerCase()) && l.topics[0] === OF) || (l.address.toLowerCase() === BLUR && BT.includes(l.topics[0]))) && l.data.toLowerCase().includes(c.slice(2)))); }
  // every window of 12 consecutive transfer txs = what the runner hands in at some moment of the day
  const counts: number[] = []; for (let i = 12; i <= txs.length; i++) counts.push(sale.slice(i - 12, i).filter(Boolean).length);
  const short = counts.filter((n) => n < 3).length;
  // in time: the runner's 20-minute evidence cadence (100 blocks); at each point the latest 12 transfer txs at or before it, within 7,200 blocks
  const pts: boolean[] = []; for (let b = head - 7200; b <= head; b += 100) { const idx = blk.map((x, i) => [x, i]).filter(([x]) => x <= b && x >= b - 7200).map(([, i]) => i).slice(-12); pts.push(idx.filter((i) => sale[i]).length >= 3); }
  let run = 0, longest = 0; for (const ok of pts) { run = ok ? 0 : run + 1; longest = Math.max(longest, run); }
  const timeShort = pts.filter((x) => !x).length;
  console.log(JSON.stringify({ collection: name, head, transferTxsIn2Days: txs.length, saleTxsIn2Days: sale.filter(Boolean).length, windows: counts.length, windowsUnder3Sales: short, shareUnder3: +(short / Math.max(1, counts.length)).toFixed(3), min: Math.min(...counts), max: Math.max(...counts), evidencePoints20min: pts.length, pointsWithNoPost: timeShort, longestNoPostRunMinutes: longest * 20 }));
}
