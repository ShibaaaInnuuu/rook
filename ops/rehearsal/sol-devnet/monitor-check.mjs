// The monitor's own read of the rehearsal canister's Solana line (docs/v9/rehearsal-sol-devnet.md, checklist item 10): the anonymous
// `sol_line` query through monitor/src/ic.mjs (the agent checks the node's signature) and the alerts monitor/src/ethnft.mjs raises on it.
//   node ops/rehearsal/sol-devnet/monitor-check.mjs <canister id> [--host https://icp-api.io] [--local]
// → one JSON line: { line: <sol_line as the monitor reads it>, alerts: [...] }
// W15 (the canister-only rehearsal, `--books`): the books home's four views the monitor pages (monitor/src/books.mjs: treasury_status,
// pool_desks, coins_log, desk_status), every listed collection's books_view and the pinned collections, and `booksAlerts` on them.
//   node ops/rehearsal/sol-devnet/monitor-check.mjs <canister id> --books [--host <url>] [--local]
// → one JSON line: { treasury, desks, coins, desk, collections, pinned, books: { <coll>: <books_view> }, alerts: [...] }
const root = new URL("../../../", import.meta.url).pathname;
const { icClient } = await import(root + "monitor/src/ic.mjs");
const args = process.argv.slice(2);
const id = args[0];
if (!id) throw new Error("usage: monitor-check.mjs <canister id> [--books] [--host <url>] [--local]");
const host = args.includes("--host") ? args[args.indexOf("--host") + 1] : "https://icp-api.io";
const ic = await icClient({ host, timelock: id, local: args.includes("--local") });
const json = (v) => JSON.stringify(v, (_, x) => (typeof x === "bigint" ? x.toString() : x)) + "\n";
const nowS = Math.floor(Date.now() / 1000);
if (args.includes("--books")) {
  const { booksAlerts } = await import(root + "monitor/src/books.mjs");
  const [treasury, desks, coins, desk, collections, pinned] = await Promise.all([
    ic.treasuryStatus(id), ic.poolDesks(id), ic.coinsLog(id), ic.deskStatus(id), ic.booksCollections(id), ic.coinsCollections(id)]);
  const books = {};
  for (const [coll] of collections) books[coll] = (await ic.booksView(id, coll))[0] ?? null;
  const alerts = booksAlerts({ buyer: id, treasury, desks, coins, desk, nowS });
  process.stdout.write(json({ treasury, desks, coins, desk, collections, pinned, books, alerts }));
} else {
  const { solLineAlerts } = await import(root + "monitor/src/ethnft.mjs");
  const raw = await ic.solLine(id);
  const line = Array.isArray(raw) ? raw[0] ?? null : raw;
  const alerts = solLineAlerts({ buyer: id, line, nowS });
  process.stdout.write(json({ line, alerts }));
}
