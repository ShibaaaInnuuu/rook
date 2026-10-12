#!/usr/bin/env node
// fixes-R3-services.md §1.5: how long the holder index (src/holders.mjs) takes on the REAL 4663 providers, read-only. There is no
// Rook stack on 4663 yet, so a live ERC-20 stands in for a coin: it is "launched" `--hours` before the head, back-filled from
// there on each provider, then brought forward once more after `--wait` seconds (one poll's new blocks), and a root is taken at a
// snapshot time an hour ago (the header binary search, then the local replay; a stand-in that predates its "launch" can have
// negative balances there, which is reported, not an error of the index). Nothing is written to any chain.
//   node monitor/scripts/measure-index.mjs --coin 0x… [--hours 2] [--wait 60] [--provider url[,span]]…
import { host, rpc } from "../src/abi.mjs";
import { HolderIndex } from "../src/holders.mjs";

const args = process.argv.slice(2), opt = (k, d) => { const i = args.indexOf(k); return i >= 0 ? args[i + 1] : d; };
const coin = opt("--coin");
if (!coin) { console.error("--coin <an ERC-20 on 4663> is required"); process.exit(2); }
const hours = Number(opt("--hours", "2")), wait = Number(opt("--wait", "60")), budget = Number(opt("--budget", "600"));
const given = args.flatMap((a, i) => (a === "--provider" ? [args[i + 1]] : []));
const providers = (given.length ? given : ["https://rpc.mainnet.chain.robinhood.com,10000", "https://robinhood.drpc.org,100", "https://rpc.ordofi.network,10000"])
  .map((x) => { const [url, span] = x.split(","); return { url, span: Number(span ?? 10_000) }; });

const head0 = Number(BigInt(await rpc(providers[0].url, "eth_blockNumber", [])));
const t0 = Number(BigInt((await rpc(providers[0].url, "eth_getBlockByNumber", [`0x${head0.toString(16)}`, false])).timestamp));
const launch = head0 - Math.round(hours * 3600 * 9.9);
console.log(`coin ${coin}; history ${hours} h (from block ${launch}, head ${head0}); ${providers.length} provider(s)`);
const counting = (url) => { const n = { getLogs: 0, headers: 0, other: 0 }; return { n, http: (u, init) => { const m = JSON.parse(init.body).method; if (m === "eth_getLogs") n.getLogs++; else if (m === "eth_getBlockByNumber") n.headers++; else n.other++; return fetch(u, init); } }; };

const rows = await Promise.all(providers.map(async (p) => {
  const { n, http } = counting(p.url);
  const ix = new HolderIndex({ url: p.url, span: p.span, factories: [], fromBlock: launch, confirmations: 20, http, retentionBlocks: Number.MAX_SAFE_INTEGER });
  ix.addCoin(coin, { launchBlock: launch });
  const out = { provider: host(p.url), span: p.span };
  let t = Date.now();
  let r = await ix.update({ deadline: Date.now() + budget * 1000 });
  out.backfill = { s: (Date.now() - t) / 1000, getLogs: n.getLogs, upTo: ix.s.coins[coin.toLowerCase()].cursor, safe: r.safe, error: r.error, span: ix.summary().span,
    transfers: ix.s.coins[coin.toLowerCase()].logs.length };
  await new Promise((ok) => setTimeout(ok, wait * 1000));
  const before = n.getLogs; t = Date.now();
  r = await ix.update({ deadline: Date.now() + 60_000 });
  out.poll = { s: (Date.now() - t) / 1000, getLogs: n.getLogs - before, newBlocks: r.safe !== null && out.backfill.safe !== null ? r.safe - out.backfill.safe : null, error: r.error };
  const h0 = n.headers; t = Date.now();
  try {
    const root = await ix.rootAt({ coin, snapshotAt: BigInt(t0 - 3600), fromBlock: launch, head: ix.s.coins[coin.toLowerCase()].cursor, exclusions: [] });
    out.root = { s: (Date.now() - t) / 1000, headers: n.headers - h0, block: root.block, holders: root.holders };
  } catch (err) { out.root = { s: (Date.now() - t) / 1000, error: String(err.message).slice(0, 160) }; }
  return out;
}));
for (const r of rows) console.log(JSON.stringify(r));
