import { readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { decodeFunctionResult, encodeFunctionData, parseAbi, type Hex } from "viem";
import { balancesOf, serialise, snapshot, type Kind } from "./core.js";
import { blockTime, call, lastBlockAtOrBefore, rpc, transferLogs } from "./rpc.js";
import { giveawayExclusions } from "./giveaway.js";
import { readExclusions, stackExclusions, type StackLike } from "./stack.js";

/**
 * docs/snapshot-v1.md (and its "Version 2"). Usage:
 *   npm run snapshot -- --coin 0x… (--block N | --time T) --kind weighted|per-wallet|pro-rata [--total N] [--min-balance WEI]
 *                       [--from-block N] [--exclusions exclusions.json] [--rpc URL] [--out file.json] [--giveaway 0xREGISTRY]
 *                       [--stack deployments/stack-4663.json --slug frogs]
 *   npm run snapshot -- --vault 0xRAFFLE --draw-id N --stack deployments/stack-4663.json --slug frogs [--rpc URL] [--exclusions …]
 * --time T (v2): the block is the last one stamped ≤ T, and the weighted tree carries the sentinel leaf (totalTickets, T).
 * --stack/--slug: a coin on our own curve; the exclusions are the stack's, derived exactly as the keeper derives them.
 * --vault/--draw-id: recompute a SweepVault draw: its coin and snapshot time are read from the chain, the tree is rebuilt, and the
 *   root is compared with the draw's. Exit 3 when they differ (the monitor's check; the owner or guardian can cancel until the pin).
 * --giveaway adds the etched-side exclusions for a PROTOCOL TOKEN giveaway, read from the registry. docs/token/protocol-token-v1.md §2.1.
 * Reads the public RPC. Sends nothing; there is nothing in this package that could.
 */
const arg = (name: string) => { const i = process.argv.indexOf(`--${name}`); return i >= 0 ? process.argv[i + 1] : undefined; };
const fail = (m: string): never => { console.error(m); process.exit(2); };

const url = arg("rpc") ?? "https://rpc.mainnet.chain.robinhood.com";
const VAULT = parseAbi([
  "struct Draw { uint8 stage; address coin; uint64 challengeEnds; uint64 challengeEndsBlock; uint64 round; uint64 pinnedAt; uint64 pinnedAtBlock; uint64 resolvedAt; uint64 resolvedAtBlock; bytes32 root; uint256 totalTickets; uint256 winningTicket; address winner; string uri; uint64 seedRound; uint64 announcedAt; uint64 announcedAtBlock; uint64 snapshotAt; }",
  "function draw(uint256 id) view returns (Draw)",
  "function routerOf(address coin) view returns (address)",
  // review2 N-2: a draw's seed origin (itself, or the draw voided or cancelled before its pin whose seed round and snapshot time it carries)
  "function seedOrigin(uint256 id) view returns (uint256)",
]);
// `Empty` (fixes-R3-carry): opened over a snapshot with no eligible holder (the sentinel alone) and closed with no winner
const STAGES = ["None", "Open", "Pinned", "Resolved", "Claimed", "Cancelled", "Voided", "Announced", "Empty"];

// a draw to recompute: its coin and time come from the chain, never from the keeper's file
const vault = arg("vault")?.toLowerCase(), drawId = arg("draw-id");
let onchain: { root: Hex; totalTickets: bigint; snapshotAt: bigint; coin: string } | null = null;
if (vault || drawId) {
  if (!vault || !drawId || !/^\d+$/.test(drawId)) fail("--vault and --draw-id go together");
  const d = decodeFunctionResult({ abi: VAULT, functionName: "draw", data: await call(url, vault!, encodeFunctionData({ abi: VAULT, functionName: "draw", args: [BigInt(drawId!)] })) as Hex });
  const stage = STAGES[Number(d.stage)] ?? String(d.stage);
  // the snapshot time is the vault's (openDraw fixes it from the seed round, hashed with the seed ORIGIN, review2 N-2): never
  // computed here from announcedAt. A draw ended before it opened has none; its seed passes to the coin's next draw.
  if (d.snapshotAt === 0n) fail(["Voided", "Cancelled"].includes(stage)
    ? `draw ${drawId} was ${stage.toLowerCase()} before it opened: it has no snapshot. Its seed round and snapshot time pass to the coin's next draw (the vault's carriedSeed / SeedCarried): recompute that draw once it is open`
    : `draw ${drawId} has no snapshot time yet (stage ${stage}): it is not open`);
  const origin = await call(url, vault!, encodeFunctionData({ abi: VAULT, functionName: "seedOrigin", args: [BigInt(drawId!)] }))
    .then((x) => decodeFunctionResult({ abi: VAULT, functionName: "seedOrigin", data: x as Hex }) as bigint, () => BigInt(drawId!));    // a vault from before R3
  if (origin !== BigInt(drawId!)) console.error(`draw ${drawId} carries draw ${origin}'s seed round and snapshot time (review2 N-2); the time below is the vault's`);
  if (d.totalTickets === 0n) console.error(`draw ${drawId} (stage ${stage}) was opened EMPTY: the keeper says no eligible holder had a whole coin at ${d.snapshotAt} (the tree is the sentinel alone); rebuilt below`);
  onchain = { root: d.root, totalTickets: d.totalTickets, snapshotAt: d.snapshotAt, coin: d.coin.toLowerCase() };
}
const kind = (onchain ? "weighted" : arg("kind")) as Kind;
if (!["weighted", "per-wallet", "pro-rata"].includes(kind)) fail("--kind must be weighted, per-wallet or pro-rata");
const coin = (onchain?.coin ?? arg("coin") ?? fail("missing --coin")).toLowerCase();
const time = onchain ? onchain.snapshotAt : arg("time") !== undefined ? BigInt(arg("time")!) : null;
if (time !== null && kind !== "weighted") fail("--time is for weighted draws (the sentinel commits totalTickets and the time)");

const stackFile = arg("stack");
const stack = stackFile ? JSON.parse(readFileSync(stackFile, "utf8")) as StackLike & { fromBlock?: number; deployedAtBlock?: number } : null;
const head = BigInt(await rpc<string>(url, "eth_blockNumber", []));
// a coin on our own curve cannot have moved before its stack was deployed
const fromBlock = BigInt(arg("from-block") ?? String(stack?.fromBlock ?? stack?.deployedAtBlock ?? 0));
const block = time !== null ? await lastBlockAtOrBefore(blockTime(url), head, time, fromBlock) : BigInt(arg("block") ?? fail("missing --block (or --time)"));
if (block > head) fail(`block ${block} does not exist yet (head ${head})`);

const file = arg("exclusions");
const listed: string[] = file ? readExclusions(file) : [];
let exclusions: string[];
if (stack) {
  // a coin on our own curve: the stack's contracts, the coin's curve (the factory's routerOf), and the listed addresses
  const slug = arg("slug") ?? fail("--stack needs --slug");
  const c = stack.collections.find((x) => x.slug === slug) ?? fail(`the stack names no collection ${slug}`);
  const curve = (decodeFunctionResult({ abi: VAULT, functionName: "routerOf", data: await call(url, c.factory, encodeFunctionData({ abi: VAULT, functionName: "routerOf", args: [coin as Hex] })) as Hex }) as string).toLowerCase();
  exclusions = stackExclusions(stack, slug, coin, curve, listed);
} else {
  if (onchain) fail("--vault/--draw-id needs --stack and --slug: a draw's exclusions are its stack's");
  // a Pons coin: what the Pons factory says about it (read at latest; none of it changes)
  const FACTORY = "0x7ed598bcef8bd9edd8c97a195c6d13f40801ec7e";
  const word = (hex: string, i: number) => `0x${hex.slice(2 + i * 64 + 24, 2 + (i + 1) * 64)}`.toLowerCase();
  const addrCall = async (to: string, selector: string) => word(await call(url, to, selector), 0);
  const launched = await call(url, FACTORY, `0x3cf28b5a${coin.slice(2).padStart(64, "0")}`);   // getLaunchedToken(address)
  const derived = ["0x0000000000000000000000000000000000000000", "0x000000000000000000000000000000000000dead", coin, word(launched, 1), FACTORY,
    await addrCall(FACTORY, "0x6651812c").catch(() => ""),   // memeHook()
    await addrCall(FACTORY, "0xc4b7de97").catch(() => ""),   // feeEscrow()
  ].filter((a) => /^0x[0-9a-f]{40}$/.test(a));
  const registry = arg("giveaway");
  const fromRegistry = registry ? await giveawayExclusions((to, data) => call(url, to, data), registry, coin) : [];
  if (registry) console.log(`giveaway exclusions from ${registry}: ${fromRegistry.join(" ")}`);
  exclusions = [...new Set([...derived, ...fromRegistry, ...listed])];
}

const decimals = Number(BigInt(await call(url, coin, "0x313ce567")));
const logs = await transferLogs(url, coin, fromBlock, block);
const out = snapshot(balancesOf(logs), { coin, block: String(block), kind, decimals, minBalance: arg("min-balance"), total: arg("total"), exclusions,
  ...(time !== null ? { snapshotAt: String(time) } : {}) });
const text = serialise(out);
const path = arg("out") ?? `snapshot-${coin}-${block}-${kind}.json`;
writeFileSync(path, text);
console.log(`root ${out.root}`);
console.log(out.totalTickets !== undefined ? `totalTickets ${out.totalTickets}` : `allocated ${out.allocated} remainder ${out.remainder}`);
if (time !== null) console.log(`snapshot time ${time}: block ${block}, the last one stamped at or before it; sentinel leaf ${out.sentinel?.leaf}`);
console.log(`entries ${out.holders}; transfers read ${logs.length}; file ${path}; sha256 ${createHash("sha256").update(text).digest("hex")}`);
if (onchain) {
  const same = out.root.toLowerCase() === onchain.root.toLowerCase() && BigInt(out.totalTickets ?? "0") === onchain.totalTickets;
  console.log(same ? `draw ${drawId}: the root and the ticket count on chain are the ones rebuilt here${onchain.totalTickets === 0n ? " (empty: no eligible holder at the snapshot, no winner)" : ""}`
    : onchain.totalTickets === 0n
      ? `FALSE EMPTY: draw ${drawId} was opened with no tickets, but ${out.holders} holders had ${out.totalTickets} tickets at the snapshot (rebuilt root ${out.root}). Pause the keeper and cancel the draw before its close: the cancel carries the same snapshot time (fixes-R3-carry)`
      : `ROOT MISMATCH: draw ${drawId} on chain has root ${onchain.root} over ${onchain.totalTickets} tickets; rebuilt: ${out.root} over ${out.totalTickets}`);
  process.exit(same ? 0 : 3);
}
