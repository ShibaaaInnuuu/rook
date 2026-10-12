import { decodeFunctionData, encodeAbiParameters, encodeEventTopics, encodeFunctionResult, getAddress, type Abi, type Hex } from "viem";
import type { Rpc, Fetch } from "../src/rpc.js";
import { CURVE_READS, ERC721_READS, FACTORY_EVENTS, FACTORY_READS, NFT_WRAPPER_EVENTS, OFFER_BOARD_EVENTS, OFFER_BOARD_READS, SWEEP_EVENTS, SWEEP_READS, TOKEN_READS, WRAPPER_EVENTS, WRAPPER_READS } from "../src/wrapped/abis.js";
import { I_FLOOR_MODULE, I_KEEPER_SOURCE, I_LOCK_PROTOCOL, I_TREASURY, PROTOCOL } from "../src/wrapped/floorModule.js";

/**
 * An in-memory world for docs/v5/ux-and-reserves.md: a 4663 node that answers eth_getLogs from encoded events and eth_call from
 * a table of views, and a `fetch` that answers ord, the two esplora explorers and EVM providers from tables. The reserve service
 * runs against it unchanged; the fixture writer uses it to build indexer/fixtures/wrapped. Nothing here touches a network.
 */
export const T0 = 1_790_000_000;
const tsOf = (block: number) => T0 + block * 2;

type Log = { address: string; topics: Hex[]; data: Hex; blockNumber: Hex; blockHash: Hex; transactionHash: Hex; logIndex: Hex };
const hx = (n: number | bigint) => `0x${BigInt(n).toString(16)}` as Hex;

export class World {
  head = 1_000;
  logs: Log[] = [];
  views = new Map<string, unknown>();        // `${address}|${fn}|${args}` -> result
  balances = new Map<string, bigint>();      // eth_getBalance
  failing = new Set<string>();               // "rpc:<method>" or "http:<url substring>"
  ord = new Map<string, { address: string; satpoint: string } | "404">();
  esplora = new Map<string, { pays: string; spent: boolean; height: number }>();   // `${txid}:${vout}`
  owners = new Map<string, Map<string, string | "revert">>();                        // providerUrl -> tokenId -> owner
  evmHead = 20_000_000;
  evmChainId = 1;
  private n = 0;

  event(address: string, abi: Abi, eventName: string, args: Record<string, unknown>, block: number): Hex {
    const ev = abi.find((x) => x.type === "event" && x.name === eventName) as unknown as { inputs: { name: string; type: string; indexed?: boolean }[] };
    const topics = encodeEventTopics({ abi, eventName, args } as never) as Hex[];
    const plain = ev.inputs.filter((i) => !i.indexed);
    const data = encodeAbiParameters(plain as never, plain.map((i) => args[i.name]) as never);
    const tx = `0x${(++this.n).toString(16).padStart(64, "0")}` as Hex;
    this.logs.push({ address: address.toLowerCase(), topics, data, blockNumber: hx(block), blockHash: `0x${"b".repeat(64)}`, transactionHash: tx, logIndex: hx(this.n) });
    return tx;
  }
  /** address arguments are keyed as viem decodes them (checksummed), so pass them through `getAddress` or let this do it */
  view(address: string, fn: string, result: unknown, args: unknown[] = []) {
    const norm = args.map((a) => (typeof a === "string" && /^0x[0-9a-fA-F]{40}$/.test(a) ? getAddress(a) : String(a)));
    this.views.set(`${address.toLowerCase()}|${fn}|${norm.join(",")}`, result);
  }

  rpc: Rpc = async <T,>(method: string, params: unknown[]): Promise<T> => {
    if (this.failing.has(`rpc:${method}`)) throw new Error(`${method}: simulated failure`);
    if (method === "eth_blockNumber") return hx(this.head) as T;
    if (method === "eth_chainId") return hx(4663) as T;
    if (method === "eth_getBlockByNumber") return { timestamp: hx(tsOf(Number(BigInt(params[0] as string)))) } as T;
    if (method === "eth_getBalance") return hx(this.balances.get(String(params[0]).toLowerCase()) ?? 0n) as T;
    if (method === "eth_getLogs") {
      const q = params[0] as { address: string; topics: Hex[][]; fromBlock: Hex; toBlock: Hex };
      const want = new Set(q.topics[0].map((t) => t.toLowerCase()));
      return this.logs.filter((l) => l.address === q.address.toLowerCase() && want.has(l.topics[0].toLowerCase())
        && BigInt(l.blockNumber) >= BigInt(q.fromBlock) && BigInt(l.blockNumber) <= BigInt(q.toBlock)) as T;
    }
    if (method === "eth_call") {
      const { to, data } = params[0] as { to: string; data: Hex };
      const abi = [...WRAPPER_READS, ...FACTORY_READS, ...CURVE_READS, ...TOKEN_READS, ...SWEEP_READS, ...OFFER_BOARD_READS, ...I_FLOOR_MODULE, ...I_TREASURY, ...I_LOCK_PROTOCOL, ...I_KEEPER_SOURCE] as Abi;
      const { functionName, args } = decodeFunctionData({ abi, data });
      const key = `${to.toLowerCase()}|${functionName}|${(args ?? []).map(String).join(",")}`;
      if (!this.views.has(key) || this.failing.has(`call:${key}`)) throw new Error(`eth_call: execution reverted (${key})`);
      return encodeFunctionResult({ abi, functionName, result: this.views.get(key) } as never) as T;
    }
    throw new Error(`the world does not answer ${method}`);
  };

  fetch: Fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    const url = String(input);
    const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
    for (const f of this.failing) if (f.startsWith("http:") && url.includes(f.slice(5))) return json({ error: "simulated outage" }, 503);
    let m: RegExpMatchArray | null;
    if ((m = url.match(/\/r\/inscription\/(.+)$/))) { const o = this.ord.get(m[1]); return !o || o === "404" ? json({ error: "not found" }, 404) : json({ id: m[1], ...o }); }
    if ((m = url.match(/\/tx\/([0-9a-f]{64})\/outspend\/(\d+)$/))) { const o = this.esplora.get(`${m[1]}:${m[2]}`); return o ? json({ spent: o.spent }) : json({}, 404); }
    if ((m = url.match(/\/tx\/([0-9a-f]{64})$/))) {
      const outs = [...this.esplora.entries()].filter(([k]) => k.startsWith(m![1]));
      if (!outs.length) return json({}, 404);
      const vout: { scriptpubkey_address: string }[] = [];
      for (const [k, v] of outs) vout[Number(k.split(":")[1])] = { scriptpubkey_address: v.pays };
      return json({ vout: Array.from(vout, (x) => x ?? { scriptpubkey_address: "" }), status: { confirmed: true, block_height: outs[0][1].height } });
    }
    // an EVM provider: JSON-RPC over POST
    const owners = this.owners.get(url);
    if (owners && init?.body) {
      const q = JSON.parse(String(init.body)) as { id: number; method: string; params: unknown[] };
      const ok = (result: unknown) => json({ jsonrpc: "2.0", id: q.id, result });
      if (q.method === "eth_chainId") return ok(hx(this.evmChainId));
      if (q.method === "eth_blockNumber") return ok(hx(this.evmHead));
      if (q.method === "eth_getBlockByNumber") return ok({ timestamp: hx(T0 + 5_000) });
      if (q.method === "eth_call") {
        const { data } = q.params[0] as { data: Hex };
        const { args } = decodeFunctionData({ abi: ERC721_READS, data });
        const o = owners.get(String(args![0]));
        if (!o || o === "revert") return json({ jsonrpc: "2.0", id: q.id, error: { message: "execution reverted" } });
        return ok(encodeFunctionResult({ abi: ERC721_READS, functionName: "ownerOf", result: o as Hex }));
      }
    }
    return json({ error: `the world has no answer for ${url}` }, 404);
  }) as Fetch;
}

// ---- the two collections of docs/v3/curve.md's worked examples -------------------------------------------------------------------

export const FROGS = {
  wrapper: "0x1000000000000000000000000000000000000001", factory: "0x1000000000000000000000000000000000000002", queue: "0x1000000000000000000000000000000000000003",
  lock: "0x1000000000000000000000000000000000000004", router: "0x1000000000000000000000000000000000000005",
  // docs/v5/floor-module.md: the module is the factory's queue; the treasury it names; docs/v6: the offer board; the YOYO vault
  module: "0x1000000000000000000000000000000000000003", treasury: "0x1000000000000000000000000000000000000006", board: "0x1000000000000000000000000000000000000007",
  yoyoVault: "0x1000000000000000000000000000000000000008", custodian: "0x1000000000000000000000000000000000000c05", ops: "0x1000000000000000000000000000000000000c06",
  custody: "bc1pfixturecustodyfrogs0000000000000000000000000000000000000q",
  alien: "0x1000000000000000000000000000000000000a11", alienCurve: "0x1000000000000000000000000000000000000a12",
  beta: "0x1000000000000000000000000000000000000b11", betaCurve: "0x1000000000000000000000000000000000000b12",
  gamma: "0x1000000000000000000000000000000000000c11", gammaCurve: "0x1000000000000000000000000000000000000c12",
} as const;
export const APES = {
  wrapper: "0x2000000000000000000000000000000000000001", factory: "0x2000000000000000000000000000000000000002", queue: "0x2000000000000000000000000000000000000003",
  collection: "0xbc4ca0eda7647a8ab7c2061c2e118a18a936f13d", custodian: "0x2000000000000000000000000000000000000c05",
  coinA: "0x2000000000000000000000000000000000000a11", coinACurve: "0x2000000000000000000000000000000000000a12",
  coinB: "0x2000000000000000000000000000000000000b11", coinBCurve: "0x2000000000000000000000000000000000000b12",
} as const;
export const PROVIDERS = ["https://eth.fixture-provider-a.invalid", "https://eth.fixture-provider-b.invalid"];
const E = 10n ** 18n;
const insc = (n: number) => `${n.toString(16).padStart(64, "f")}i0`;
const btx = (n: number) => n.toString(16).padStart(64, "a");
const DEPOSIT_FLAG = 1n << 255n;

/**
 * Frogs: ALIEN (5 ETH, the first worked example) graduated and its queue bought six frogs; a holder deposited a seventh; three
 * whole tokens were redeemed — one delivered, one resolved and waiting for delivery, one pinned. BETA is still on its curve.
 * Apes: two 5 ETH coins queued FIFO and one ape was bought (the "two normal coins" example). No Floor Module on either.
 */
export function buildWorld(): World {
  const w = new World(), holder = "0x3000000000000000000000000000000000000001", redeemer = "0x3000000000000000000000000000000000000002";
  const now = T0 + w.head * 2;
  // ---- frogs
  w.event(FROGS.factory, FACTORY_EVENTS, "Launched", { coin: FROGS.alien, curve: FROGS.alienCurve, creator: holder, policy: 0, raise: 5n * E, sweepOnly: false, early: false, opening: false }, 10);
  w.event(FROGS.factory, FACTORY_EVENTS, "Launched", { coin: FROGS.beta, curve: FROGS.betaCurve, creator: redeemer, policy: 1, raise: 5n * E, sweepOnly: false, early: false, opening: false }, 300);
  for (let i = 0; i < 6; i++) w.event(FROGS.wrapper, WRAPPER_EVENTS, "Deposited", { frogId: DEPOSIT_FLAG | BigInt(i), inscriptionId: insc(i), txid: `0x${btx(i)}`, vout: 0, to: FROGS.queue }, 100 + i * 10);
  w.event(FROGS.wrapper, WRAPPER_EVENTS, "Deposited", { frogId: DEPOSIT_FLAG | 6n, inscriptionId: insc(6), txid: `0x${btx(6)}`, vout: 1, to: holder }, 400);
  const tb = "bc1pfixtureredeemerdestination00000000000000000000000000000qq";
  w.event(FROGS.wrapper, WRAPPER_EVENTS, "Redeemed", { ticketId: 0n, owner: redeemer, round: 5_000_100n, destination: tb }, 500);
  w.event(FROGS.wrapper, WRAPPER_EVENTS, "Resolved", { ticketId: 0n, acquisitionId: DEPOSIT_FLAG | 2n, deliverBy: BigInt(T0 + 510 * 2 + 7 * 86_400) }, 510);
  w.event(FROGS.wrapper, WRAPPER_EVENTS, "Delivered", { ticketId: 0n, acquisitionId: DEPOSIT_FLAG | 2n, txid: `0x${btx(90)}` }, 600);
  w.event(FROGS.wrapper, WRAPPER_EVENTS, "Redeemed", { ticketId: 1n, owner: redeemer, round: 5_000_300n, destination: tb }, 700);
  w.event(FROGS.wrapper, WRAPPER_EVENTS, "Resolved", { ticketId: 1n, acquisitionId: DEPOSIT_FLAG | 4n, deliverBy: BigInt(now + 6 * 86_400) }, 720);
  w.event(FROGS.wrapper, WRAPPER_EVENTS, "Redeemed", { ticketId: 2n, owner: holder, round: 5_000_500n, destination: tb }, 990);
  // GAMMA: a YOYO coin (docs/v5/policies-and-fees.md §1) at the 5% tier, launched 200 s ago so its launch fee is still decaying
  w.event(FROGS.factory, FACTORY_EVENTS, "Launched", { coin: FROGS.gamma, curve: FROGS.gammaCurve, creator: holder, policy: 2, raise: 5n * E, sweepOnly: false, early: false, opening: false }, 900);
  // the module holds 2.5 ETH-backed units beside the 4 item-backed ones (6 held − 2 in flight): supply is 6.5
  w.view(FROGS.wrapper, "totalSupply", 6_500_000_000_000_000_000n); w.view(FROGS.wrapper, "held", 6n); w.view(FROGS.wrapper, "openTickets", 2n); w.view(FROGS.wrapper, "symbol", "wFROG");
  w.view(FROGS.factory, "freshCap", [646_000_000_000_000_000n, true]);
  w.view(FROGS.factory, "raffleVault", "0x0000000000000000000000000000000000000000"); w.view(FROGS.factory, "burnVault", "0x0000000000000000000000000000000000000000");
  w.view(FROGS.factory, "yoyoVault", getAddress(FROGS.yoyoVault));
  // docs/v7/phase1-nft-pooled.md §2: this collection's own graduation targets (BAYC's 9 ETH, say)
  w.view(FROGS.factory, "normalRaise", 9n * E); w.view(FROGS.factory, "earlyRaise", 9n * E); w.view(FROGS.factory, "earlyUntilEth", 20n * E); w.view(FROGS.factory, "maxRaise", 30n * E);
  w.view(FROGS.factory, "feeOf", 100n, [FROGS.alien]); w.view(FROGS.factory, "feeOf", 300n, [FROGS.beta]); w.view(FROGS.factory, "feeOf", 500n, [FROGS.gamma]);
  w.view(FROGS.alien, "name", "Alien"); w.view(FROGS.alien, "symbol", "ALIEN"); w.view(FROGS.alienCurve, "state", 2); w.view(FROGS.alienCurve, "progress", [800_000_000n * E, 5n * E]);
  w.view(FROGS.beta, "name", "Beta Frog"); w.view(FROGS.beta, "symbol", "BETA"); w.view(FROGS.betaCurve, "state", 0); w.view(FROGS.betaCurve, "progress", [372_300_000n * E, 1_200_000_000_000_000_000n]);
  w.view(FROGS.gamma, "name", "Gamma Frog"); w.view(FROGS.gamma, "symbol", "GAMMA"); w.view(FROGS.gammaCurve, "state", 2); w.view(FROGS.gammaCurve, "progress", [800_000_000n * E, 5n * E]);
  for (const [curve, fee, at, extra] of [[FROGS.alienCurve, 100n, tsOf(10), 0n], [FROGS.betaCurve, 300n, tsOf(300), 0n], [FROGS.gammaCurve, 500n, tsOf(900), 3_333n]] as const) {
    w.view(curve, "feeBps", fee); w.view(curve, "launchFeeBps", 5_000n); w.view(curve, "launchFeeDecay", 600n); w.view(curve, "launchedAt", BigInt(at)); w.view(curve, "launchExtraBps", extra);
  }
  // the YOYO vault: GAMMA had two items relisted at the 1.1× markup; one sold, its proceeds bought GAMMA back and burned it
  w.event(FROGS.yoyoVault, SWEEP_EVENTS, "Relisted", { coin: FROGS.gamma, ticketId: 3n, ask: 710_600_000_000_000_000n }, 950);
  w.event(FROGS.yoyoVault, SWEEP_EVENTS, "Relisted", { coin: FROGS.gamma, ticketId: 4n, ask: 710_600_000_000_000_000n }, 960);
  w.event(FROGS.yoyoVault, SWEEP_EVENTS, "SaleRecorded", { coin: FROGS.gamma, ticketId: 3n, proceeds: 690_000_000_000_000_000n }, 980);
  w.event(FROGS.yoyoVault, SWEEP_EVENTS, "BoughtBack", { coin: FROGS.gamma, caller: holder, spent: 340_000_000_000_000_000n, burned: 2_000_000n * E }, 990);
  w.view(FROGS.yoyoVault, "listedOpen", 1n, [FROGS.gamma]); w.view(FROGS.yoyoVault, "proceeds", 350_000_000_000_000_000n, [FROGS.gamma]); w.view(FROGS.yoyoVault, "markupBps", 1_000n);
  // the floor module (docs/v5/floor-module.md §13): 2.5 ETH-backed units, a quarter of them the standing reserve, half a unit of float inventory
  const M = FROGS.module;
  w.view(M, "wrapped", getAddress(FROGS.wrapper)); w.view(M, "floor", [633_000_000_000_000_000n, BigInt(now - 600), 3_600n]); w.view(M, "fresh", true);
  w.view(M, "price", 640_000_000_000_000_000n); w.view(M, "fillBandBps", 300n); w.view(M, "fillCount", 2n);
  w.view(M, "fill", { price: 645_000_000_000_000_000n, at: BigInt(now - 3_600) }, [0n]); w.view(M, "fill", { price: 636_000_000_000_000_000n, at: BigInt(now - 20_000) }, [1n]);
  w.view(M, "spreads", [21n, 15n]); w.view(M, "spreadBps", 15n);
  w.view(M, "ethBackedUnits", 2_500_000_000_000_000_000n); w.view(M, "reserveUnits", 625_000_000_000_000_000n); w.view(M, "netUnits", 1_875_000_000_000_000_000n); w.view(M, "floatUnits", 500_000_000_000_000_000n);
  w.view(M, "escrowBalance", 1_585_000_000_000_000_000n); w.view(M, "escrow", 1_188_750_000_000_000_000n); w.view(M, "reserve", 396_250_000_000_000_000n); w.view(M, "revenue", 4_000_000_000_000_000n);
  w.view(M, "betaConvertible", 634_000_000_000_000_000n); w.view(M, "betaReserve", 634_000_000_000_000_000n);
  w.view(M, "mintCapacity", [641_344_000_000_000_000n, 3n * E]); w.view(M, "burnCapacity", [639_040_000_000_000_000n, 625_000_000_000_000_000n]);
  w.view(M, "buffer", E / 20n);
  w.view(M, "loanOutstanding", 0n); w.view(M, "treasury", getAddress(FROGS.treasury)); w.view(M, "volume7d", 3_200_000_000_000_000_000n);
  // the treasury (§13.3): one registered collection, the default 40 / 30 / 20 / 10, nothing lent
  const T = FROGS.treasury;
  w.balances.set(T, 800_000_000_000_000_000n);
  w.view(T, "reservesPot", 200_000_000_000_000_000n); w.view(T, "floatsPot", 150_000_000_000_000_000n); w.view(T, "bootstrapPot", 400_000_000_000_000_000n); w.view(T, "unallocated", 50_000_000_000_000_000n);
  w.view(T, "reservesBps", 4_000); w.view(T, "floatsBps", 3_000); w.view(T, "bootstrapBps", 2_000); w.view(T, "opsBps", 1_000); w.view(T, "ops", getAddress(FROGS.ops)); w.view(T, "bootstrapLoan", 2n * E);
  w.view(T, "totalReceived", 1_250_000_000_000_000_000n); w.view(T, "totalAllocated", 1_200_000_000_000_000_000n); w.view(T, "totalToOps", 120_000_000_000_000_000n);
  w.view(T, "totalToReserves", 280_000_000_000_000_000n); w.view(T, "totalToFloats", 210_000_000_000_000_000n); w.view(T, "totalLent", 0n); w.view(T, "totalRepaid", 0n);
  w.view(T, "collectionCount", 1n); w.view(T, "collection", getAddress(M), [0n]); w.view(T, "weights", [[3_200_000_000_000_000_000n], 3_200_000_000_000_000_000n]); w.view(T, "outstanding", 0n, [M]);
  // the lock's protocol-owned liquidity (§13.2): two positions placed, a little ETH waiting for the price guard
  const L = FROGS.lock;
  w.view(L, "pendingEth", 10_000_000_000_000_000n, [PROTOCOL]); w.view(L, "pendingWrapped", 0n, [PROTOCOL]);
  w.view(L, "coinPositions", [`0x${"1".repeat(64)}`, `0x${"2".repeat(64)}`], [PROTOCOL]);
  w.view(L, "sharedEthIn", 33n * E); w.view(L, "sharedWrappedIn", 51n * E); w.view(L, "feesInEth", 300_000_000_000_000_000n); w.view(L, "feesPaidEth", 250_000_000_000_000_000n); w.view(L, "treasury", getAddress(T));
  // the offer board (docs/v6/offer-handins.md): three hand-ins — one bought and claimed, one slashed, one open (the cheapest)
  const Bd = FROGS.board, bot1 = "0x3000000000000000000000000000000000000b01", bot2 = "0x3000000000000000000000000000000000000b02";
  const subs = [
    { submitter: bot1, at: BigInt(tsOf(600)), atBlock: 600n, status: 2, bond: 10_000_000_000_000_000n, askSats: 1_020_000n, offerHash: `0x${"a1".repeat(32)}`, inscriptionId: insc(20) },
    { submitter: bot2, at: BigInt(tsOf(700)), atBlock: 700n, status: 3, bond: 10_000_000_000_000_000n, askSats: 990_000n, offerHash: `0x${"b2".repeat(32)}`, inscriptionId: insc(21) },
    { submitter: bot1, at: BigInt(tsOf(980)), atBlock: 980n, status: 1, bond: 10_000_000_000_000_000n, askSats: 1_005_000n, offerHash: `0x${"c3".repeat(32)}`, inscriptionId: insc(22) },
  ];
  subs.forEach((x, i) => { w.event(Bd, OFFER_BOARD_EVENTS, "Submitted", { id: BigInt(i), submitter: x.submitter, offerHash: x.offerHash, askSats: x.askSats, inscriptionId: x.inscriptionId }, Number(x.atBlock)); w.view(Bd, "submissionOf", x, [BigInt(i)]); });
  w.event(Bd, OFFER_BOARD_EVENTS, "Funded", { from: holder, amount: 60_000_000_000_000_000n }, 500);
  w.event(Bd, OFFER_BOARD_EVENTS, "Claimed", { id: 0n, submitter: bot1, reward: 10_000_000_000_000_000n }, 800);
  w.event(Bd, OFFER_BOARD_EVENTS, "Slashed", { id: 1n, submitter: bot2, bond: 10_000_000_000_000_000n }, 820);
  w.view(Bd, "params", [10_000_000_000_000_000n, 10_000_000_000_000_000n, 3_600n, 86_400n, 200_000_000_000_000_000n, 64n]); w.view(Bd, "maxOpenPerSubmitter", 4n);
  w.view(Bd, "custodian", getAddress(FROGS.custodian)); w.view(Bd, "rewardPool", 60_000_000_000_000_000n); w.view(Bd, "paidToday", 10_000_000_000_000_000n);
  w.view(Bd, "submissionCount", 3n); w.view(Bd, "openCount", 1n); w.view(Bd, "openIds", [2n], [0n, 64n]); w.view(Bd, "slashable", false, [2n]); w.view(Bd, "expired", false, [2n]);
  for (let i = 0; i < 7; i++) if (i !== 2) {
    w.ord.set(insc(i), { address: FROGS.custody, satpoint: `${btx(i)}:${i === 6 ? 1 : 0}:0` });
    w.esplora.set(`${btx(i)}:${i === 6 ? 1 : 0}`, { pays: FROGS.custody, spent: false, height: 915_000 + i });
  }
  // ---- apes
  w.event(APES.factory, FACTORY_EVENTS, "Launched", { coin: APES.coinA, curve: APES.coinACurve, creator: holder, policy: 0, raise: 5n * E, sweepOnly: false, early: false, opening: false }, 20);
  w.event(APES.factory, FACTORY_EVENTS, "Launched", { coin: APES.coinB, curve: APES.coinBCurve, creator: redeemer, policy: 0, raise: 5n * E, sweepOnly: false, early: false, opening: false }, 30);
  w.event(APES.wrapper, NFT_WRAPPER_EVENTS, "Wrapped", { itemId: 0n, tokenId: 8520n, purchaseTx: `0x${"c3d3dd48".padEnd(64, "0")}`, to: APES.queue }, 200);
  w.view(APES.wrapper, "totalSupply", E); w.view(APES.wrapper, "held", 1n); w.view(APES.wrapper, "openTickets", 0n); w.view(APES.wrapper, "symbol", "wAPE");
  w.view(APES.factory, "freshCap", [5_100_000_000_000_000_000n, false]);
  w.view(APES.factory, "raffleVault", "0x0000000000000000000000000000000000000000"); w.view(APES.factory, "burnVault", "0x0000000000000000000000000000000000000000");
  w.view(APES.coinA, "name", "Ape Coin A"); w.view(APES.coinA, "symbol", "APEA"); w.view(APES.coinACurve, "state", 2); w.view(APES.coinACurve, "progress", [800_000_000n * E, 5n * E]);
  w.view(APES.coinB, "name", "Ape Coin B"); w.view(APES.coinB, "symbol", "APEB"); w.view(APES.coinBCurve, "state", 1); w.view(APES.coinBCurve, "progress", [800_000_000n * E, 5n * E]);
  for (const p of PROVIDERS) w.owners.set(p, new Map([["8520", APES.custodian]]));
  return w;
}
export const NOW = T0 + 1_000 * 2 + 30;
export const worldConfig = (maxAgeS = 900) => ({
  chainId: 4663, confirmations: 0, maxAgeS, logChunk: 250,
  collections: [
    { slug: "frogs", name: "Bitcoin Frogs", symbol: "wFROG", kind: "inscriptions" as const, description: "Bitcoin Frogs inscriptions held at one Bitcoin custody address.",
      wrapper: FROGS.wrapper, fromBlock: 1, factory: FROGS.factory, queue: FROGS.queue, lock: FROGS.lock, router: FROGS.router, floorModule: FROGS.module as `0x${string}`,
      offerBoard: FROGS.board as `0x${string}`, custody: { kind: "bitcoin" as const, address: FROGS.custody } },
    { slug: "apes", name: "Bored Ape Yacht Club", symbol: "wAPE", kind: "nft" as const, description: "Bored Apes held at the canister's own address on Ethereum.",
      wrapper: APES.wrapper, fromBlock: 1, factory: APES.factory, queue: APES.queue, floorModule: null,
      custody: { kind: "evm" as const, chainId: 1, collection: APES.collection, custodian: APES.custodian, providers: PROVIDERS } },
  ],
});
