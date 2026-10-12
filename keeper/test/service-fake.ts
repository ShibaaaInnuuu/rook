import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { decodeFunctionData, encodeAbiParameters, encodeErrorResult, encodeFunctionResult, keccak256, parseTransaction, recoverTransactionAddress, type Abi, type Hex } from "viem";
import { ticketLeaf, verify } from "../../tools/snapshot/src/core.js";
import { BEACON, ERRORS, FACTORY, MODULE, REGISTRY, VAULT, VAULT_PUBLIC, WRAPPER, WRAPPER_PUBLIC } from "../service/abis.js";

/**
 * A fake Robinhood-Chain-like node for the production keeper's and the resolver's tests: JSON-RPC over an injected `fetch`, the
 * contracts the keeper touches reduced to the rules it must respect (keeper-only through the registry and its pause, the module's
 * band, the vault's H12 step, gap and ceiling, the draw flow announce → open with the sentinel → request → resolve → claim, the
 * Wrapper's in-order tickets, the drand beacon's verify). The same fetch answers the drand relays and a snapshot mirror. Not a
 * contract test: those are forge's, and the fork run's (scripts/keeper-fork-test.sh, scripts/golive-dryrun.sh).
 */
export const A = (n: number): Hex => `0x${n.toString(16).padStart(40, "0")}` as Hex;
export const REG = A(0x1001), MOD = A(0x1002), RAFFLE = A(0x1003), BURN = A(0x1004), YOYO = A(0x1005), FACT = A(0x1006), WRAP = A(0x1007);
export const LOCK = A(0x1008), ROUTER = A(0x1009), TREAS = A(0x100a), PM = A(0x100b), DESK = A(0x100c), BEACON_ADDR = A(0x100e);
export const COIN = A(0x2001), CURVE = A(0x2002);
const E18 = 10n ** 18n;
export const GENESIS = 1_727_521_075, PERIOD = 3;
export const roundAt = (ts: number): bigint => BigInt(Math.floor((ts - GENESIS) / PERIOD) + 1);
export const timeOf = (r: bigint): number => GENESIS + (Number(r) - 1) * PERIOD;
/** The fake network's signature of a round: 64 bytes nobody else would produce. */
export const sigOf = (r: bigint): Hex => `0x${keccak256(encodeAbiParameters([{ type: "uint64" }], [r])).slice(2)}${keccak256(encodeAbiParameters([{ type: "uint64" }, { type: "string" }], [r, "g1"])).slice(2)}` as Hex;
export const randomnessOf = (sig: Hex): Hex => `0x${createHash("sha256").update(Buffer.from(sig.slice(2), "hex")).digest("hex")}` as Hex;

export type Tx = { hash: Hex; from: string; to: string; data: Hex; nonce: number; gas: bigint; maxFeePerGas: bigint; block: number | null; status: 1 | 0; replaced?: boolean };
export type FakeDraw = { stage: number; coin: Hex; challengeEnds: number; challengeEndsBlock: number; round: bigint; pinnedAt: number; pinnedAtBlock: number; resolvedAt: number;
  resolvedAtBlock: number; root: Hex; totalTickets: bigint; winningTicket: bigint; winner: Hex; uri: string; seedRound: bigint; announcedAt: number; announcedAtBlock: number; snapshotAt: number };
export type FakeTicket = { stage: number; owner: Hex; round: bigint; pinnedAt: number; pinnedAtBlock: number; deliverBy: number; acquisitionId: bigint };

export class FakeChain {
  chainId: number;
  head = 100_000;                 // one block a second: a day and more of history behind the head
  headTs = 1_790_000_000;
  baseFee = 10_000_000n;          // 0.01 gwei
  tip = 0n;
  keeper: string;
  moduleKeeper: string | null = null;
  keeperPaused = false;
  keeperCode = "0x";
  balance = 10n ** 17n;
  estimate = 100_000n;
  nonce = 0;
  txs = new Map<string, Tx>();
  sent: Hex[] = [];
  asked: string[] = [];
  /** mine each accepted transaction at once (true) or leave it pending (false) */
  mine = true;
  /** answer eth_sendRawTransaction with this JSON-RPC error */
  rejectSend: string | null = null;
  /** refuse eth_getLogs over more than this many blocks with the provider's "exceeds limit" answer (E-11) */
  logsLimitBlocks: number | null = null;
  /** answer this many calls with a JSON-RPC throttle (HTTP 200, code 429) before answering (E-04) */
  throttle = 0;
  // the module
  accepted = 0n; target = 0n; postedAt = 0; confidence = 0; maxMoveBps = 500; minConfidenceBps = 3_000;
  // the vaults: cap, capAt (ts), capAtBlock (evm), ceiling, policy, registry
  vaults: Record<string, { cap: bigint; capAt: number; capAtBlock: number; ceiling: bigint; policy: number; registry: string }>;
  units = new Map<string, bigint>();
  coins: { coin: Hex; policy: number; curve: Hex }[] = [];
  draws: FakeDraw[] = [];
  inFlight = new Map<string, number>();
  /** review2 N-2: a coin's carried seed (origin id + 1) and each draw's seed origin (SweepVault.carriedSeed / seedOrigin) */
  carried = new Map<string, number>();
  origins: number[] = [];
  transfers: { block: number; from: string; to: string; value: bigint }[] = [];
  tickets: FakeTicket[] = [];
  nextToResolve = 0;
  claimed: { id: number; holder: string }[] = [];
  // drand: the relays serve a round once the chain's clock has reached it; `relayLie` makes Cloudflare's relay answer other bytes
  relayLie = false;
  /** the mirror answers a GET of the object it was PUT (R2-S5 reads it back); false: a PUT-only mirror */
  mirrorServesPut = true;
  relayDown = new Set<string>();
  /** the snapshot mirror and the keeper's public URL: files by URL; `serveDir` answers the keeper's own /snapshots/ */
  files = new Map<string, string>();
  serveDir: string | null = null;
  puts: { url: string; auth: string | null }[] = [];

  constructor(o: { chainId?: number; keeper: string }) {
    this.chainId = o.chainId ?? 4663;
    this.keeper = o.keeper;
    const TEN = 10n * E18;          // every vault has a ceiling (C-01: none, no cap); a test that needs none sets it to 0
    this.vaults = { [RAFFLE.toLowerCase()]: { cap: 0n, capAt: 0, capAtBlock: 0, ceiling: TEN, policy: 0, registry: REG },
      [BURN.toLowerCase()]: { cap: 0n, capAt: 0, capAtBlock: 0, ceiling: TEN, policy: 1, registry: REG },
      [YOYO.toLowerCase()]: { cap: 0n, capAt: 0, capAtBlock: 0, ceiling: TEN, policy: 2, registry: REG } };
  }

  /** The EVM's block.number: 4663 reports the parent chain's (a block every 12 s); this fake's L2 makes one a second. */
  evm(n = this.head): number { return Math.floor(n / 12); }
  tsOf(n: number): number { return this.headTs - (this.head - n) * 1; }   // one second per block
  advance(seconds: number): void { this.head += seconds; this.headTs += seconds; }
  regKeeper(): string { return this.keeperPaused ? A(0) : this.keeper; }
  fresh(): boolean { return this.accepted !== 0n && this.confidence >= this.minConfidenceBps && this.headTs < this.postedAt + 3_600; }
  bounds(): [bigint, bigint] {
    if (this.accepted === 0n) return [0n, 2n ** 256n - 1n];
    const el = BigInt(Math.min(Math.max(0, this.headTs - this.postedAt), 3_600));
    const step = (this.accepted * BigInt(this.maxMoveBps) * el) / (10_000n * 3_600n);
    return [this.accepted - step, this.accepted + step];
  }
  capFresh(v: string): boolean { const s = this.vaults[v]; return !!s && s.cap !== 0n && !this.keeperPaused && this.headTs < s.capAt + 3_600; }
  seedOrigin(id: number): number { return this.origins[id] ?? id; }
  /** As SweepVault.snapshotTimeOf: hashed with, and counted back from, the draw's seed origin (review2 N-2). */
  snapshotTimeOf(id: number, rnd: Hex): number {
    const o = this.seedOrigin(id);
    const x = BigInt(keccak256(encodeAbiParameters([{ type: "bytes32" }, { type: "address" }, { type: "uint256" }, { type: "string" }], [rnd, RAFFLE, BigInt(o), "snapshot"]))) % 86_400n;
    return this.draws[o].announcedAt - 86_400 + Number(x);
  }
  /** The last block stamped at or before `t`, as the rule names it (one block a second here). */
  lastBlockAtOrBefore(t: number): number { return this.head - (this.headTs - t); }

  private revert(name: string, args: unknown[] = []): never {
    const data = encodeErrorResult({ abi: ERRORS, errorName: name as never, args: args as never });
    throw Object.assign(new Error("execution reverted"), { rpc: { code: 3, message: "execution reverted", data } });
  }
  private proven(round: bigint, sig: Hex): Hex {
    if (sig.toLowerCase() !== sigOf(round).toLowerCase() || timeOf(round) > this.headTs) this.revert("InvalidSignature");
    return randomnessOf(sig);
  }
  private drawOut(d: FakeDraw) {
    return { stage: d.stage, coin: d.coin, challengeEnds: BigInt(d.challengeEnds), challengeEndsBlock: BigInt(d.challengeEndsBlock), round: d.round, pinnedAt: BigInt(d.pinnedAt),
      pinnedAtBlock: BigInt(d.pinnedAtBlock), resolvedAt: BigInt(d.resolvedAt), resolvedAtBlock: BigInt(d.resolvedAtBlock), root: d.root, totalTickets: d.totalTickets,
      winningTicket: d.winningTicket, winner: d.winner, uri: d.uri, seedRound: d.seedRound, announcedAt: BigInt(d.announcedAt), announcedAtBlock: BigInt(d.announcedAtBlock),
      snapshotAt: BigInt(d.snapshotAt) };
  }

  /** Apply (or, for a simulation, only check) a call. */
  private exec(from: string, to: string, data: Hex, apply: boolean): Hex {
    const t = to.toLowerCase(), f = from.toLowerCase();
    const enc = (abi: Abi | readonly unknown[], fn: string, v: unknown) => encodeFunctionResult({ abi: abi as Abi, functionName: fn, result: v as never });
    if (data === "0x" || t === f) return "0x";                    // a plain transfer (a self-transfer clearing a nonce)
    if (t === REG.toLowerCase()) {
      const d = decodeFunctionData({ abi: REGISTRY, data });
      switch (d.functionName) {
        case "keeper": return enc(REGISTRY, "keeper", this.regKeeper());
        case "namedKeeper": return enc(REGISTRY, "namedKeeper", this.keeper);
        case "keeperPaused": return enc(REGISTRY, "keeperPaused", this.keeperPaused);
        case "guardian": return enc(REGISTRY, "guardian", A(0));
        default: return enc(REGISTRY, "owner", A(0xabc));
      }
    }
    if (t === BEACON_ADDR.toLowerCase()) {
      const d = decodeFunctionData({ abi: BEACON, data }) as unknown as { functionName: string; args: unknown[] };
      switch (d.functionName) {
        case "verify": { const [r, sig] = d.args as [bigint, Hex]; return enc(BEACON, "verify", this.proven(r, sig)); }
        case "roundAt": return enc(BEACON, "roundAt", roundAt(Number(d.args[0])));
        case "timeOf": return enc(BEACON, "timeOf", BigInt(timeOf(d.args[0] as bigint)));
        case "randomnessOf": return enc(BEACON, "randomnessOf", `0x${"0".repeat(64)}`);
      }
    }
    if (t === MOD.toLowerCase()) {
      const d = decodeFunctionData({ abi: MODULE as unknown as Abi, data }) as { functionName: string; args: unknown[] };
      switch (d.functionName) {
        case "floor": return enc(MODULE, "floor", [this.accepted, BigInt(this.postedAt), 3_600n]);
        case "maxMoveBps": return enc(MODULE, "maxMoveBps", this.maxMoveBps);
        case "floorBounds": return enc(MODULE, "floorBounds", this.bounds());
        case "keeper": return enc(MODULE, "keeper", this.moduleKeeper ?? this.regKeeper());
        case "fresh": return enc(MODULE, "fresh", this.fresh());
        case "accepted": return enc(MODULE, "accepted", this.accepted);
        case "minConfidenceBps": return enc(MODULE, "minConfidenceBps", BigInt(this.minConfidenceBps));
        case "confidenceBps": return enc(MODULE, "confidenceBps", this.confidence);
        case "postFloor": {
          if (f !== (this.moduleKeeper ?? this.regKeeper()).toLowerCase()) this.revert("NotKeeper");
          const [price, conf] = d.args as [bigint, number, number];
          if (price === 0n || conf > 10_000) this.revert("BadInput");
          if (apply) {
            const [lo, hi] = this.bounds();
            this.accepted = price > hi ? hi : price < lo ? lo : price; this.target = price; this.confidence = conf; this.postedAt = this.headTs;
          }
          return "0x";
        }
      }
    }
    if (t === WRAP.toLowerCase()) {
      const d = decodeFunctionData({ abi: [...WRAPPER, ...WRAPPER_PUBLIC] as Abi, data }) as unknown as { functionName: string; args: unknown[] };
      switch (d.functionName) {
        case "nextToResolve": return enc(WRAPPER, "nextToResolve", BigInt(this.nextToResolve));
        case "ticketCount": return enc(WRAPPER, "ticketCount", BigInt(this.tickets.length));
        case "beacon": return enc(WRAPPER, "beacon", BEACON_ADDR);
        case "ticket": {
          const k = this.tickets[Number(d.args[0])];
          return enc(WRAPPER, "ticket", { stage: k.stage, owner: k.owner, round: k.round, pinnedAt: BigInt(k.pinnedAt), pinnedAtBlock: BigInt(k.pinnedAtBlock), deliverBy: BigInt(k.deliverBy),
            deliverByBlock: 0n, acquisitionId: k.acquisitionId, deliveryTxid: `0x${"0".repeat(64)}`, destination: "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr" });
        }
        case "resolve": {
          const [id, sig] = d.args as [bigint, Hex];
          if (Number(id) !== this.nextToResolve) this.revert("OutOfOrder", [BigInt(this.nextToResolve)]);
          const k = this.tickets[Number(id)];
          if (!k || k.stage !== 1) this.revert("WrongStage");
          const rnd = this.proven(k.round, sig);
          if (apply) {
            k.stage = 2; k.acquisitionId = BigInt(rnd) % 7n; k.deliverBy = this.headTs + 7 * 86_400;
            this.nextToResolve = Number(id) + 1;
            while (this.nextToResolve < this.tickets.length && this.tickets[this.nextToResolve].stage === 4) this.nextToResolve++;
          }
          return "0x";
        }
      }
    }
    if (this.vaults[t]) {
      const v = this.vaults[t];
      const d = decodeFunctionData({ abi: [...VAULT, ...VAULT_PUBLIC] as Abi, data }) as unknown as { functionName: string; args: unknown[] };
      const onlyKeeper = () => { if (f !== this.regKeeper().toLowerCase()) this.revert("NotKeeper"); };
      const draw = (id: bigint) => { const x = this.draws[Number(id)]; if (!x) this.revert("WrongStage"); return x; };
      const passed = (at: number, atBlock: number) => this.headTs >= at && this.evm() >= atBlock;
      switch (d.functionName) {
        case "registry": return enc(VAULT, "registry", v.registry);
        case "policy": return enc(VAULT, "policy", v.policy);
        case "cap": return enc(VAULT, "cap", v.cap);
        case "capAt": return enc(VAULT, "capAt", BigInt(v.capAt));
        case "capAtBlock": return enc(VAULT, "capAtBlock", BigInt(v.capAtBlock));
        case "capFresh": return enc(VAULT, "capFresh", this.capFresh(t));
        case "capCeiling": return enc(VAULT, "capCeiling", v.ceiling);
        case "parentBlockSeconds": return enc(VAULT, "parentBlockSeconds", 12n);
        case "units": return enc(VAULT, "units", this.units.get(String(d.args[0]).toLowerCase()) ?? 0n);
        case "drawCount": return enc(VAULT, "drawCount", BigInt(this.draws.length));
        case "drawInFlight": return enc(VAULT, "drawInFlight", BigInt(this.inFlight.get(String(d.args[0]).toLowerCase()) ?? 0));
        case "draw": return enc(VAULT, "draw", this.drawOut(draw(d.args[0] as bigint)));
        case "beacon": return enc(VAULT, "beacon", BEACON_ADDR);
        case "seedOrigin": return enc(VAULT, "seedOrigin", BigInt(this.seedOrigin(Number(d.args[0]))));
        case "carriedSeed": return enc(VAULT, "carriedSeed", BigInt(this.carried.get(String(d.args[0]).toLowerCase()) ?? 0));
        case "snapshotTimeOf": return enc(VAULT, "snapshotTimeOf", BigInt(this.snapshotTimeOf(Number(d.args[0]), d.args[1] as Hex)));
        case "postCap": {
          onlyKeeper();
          const [cap] = d.args as [bigint];
          if (cap === 0n) this.revert("BadInput");
          if (v.ceiling === 0n) this.revert("NoCeiling");
          if (this.capFresh(t)) {
            if (this.headTs < v.capAt + 600 || this.evm() < v.capAtBlock + 50) this.revert("CapTooSoon");
            const lo = v.cap - (v.cap * 2_000n) / 10_000n, hi = v.cap + (v.cap * 2_000n) / 10_000n;
            if (cap < lo || cap > hi) this.revert("CapStep", [cap, lo, hi]);
          } else if (v.cap !== 0n) {
            const hi = v.cap + (v.cap * 2_000n) / 10_000n;
            if (cap > hi) this.revert("CapStep", [cap, 0n, hi]);
          }
          if (cap > v.ceiling) this.revert("OverCeiling", [cap, v.ceiling]);
          if (apply) { v.cap = cap; v.capAt = this.headTs; v.capAtBlock = this.evm(); }
          return "0x";
        }
        case "announceDraw": {
          onlyKeeper();
          if (v.policy !== 0) this.revert("WrongPolicy");
          const coin = String(d.args[0]).toLowerCase() as Hex;
          if (this.inFlight.get(coin)) this.revert("DrawInFlight");
          const u = this.units.get(coin) ?? 0n;
          if (u < E18) this.revert("Insufficient");
          if (apply) {
            this.units.set(coin, u - E18);
            const carried = this.carried.get(coin) ?? 0;          // review2 N-2: the seed round and window of a draw voided or cancelled before its pin
            this.origins[this.draws.length] = carried ? carried - 1 : this.draws.length;
            this.draws.push({ stage: 7, coin, challengeEnds: 0, challengeEndsBlock: 0, round: 0n, pinnedAt: 0, pinnedAtBlock: 0, resolvedAt: 0, resolvedAtBlock: 0, root: `0x${"0".repeat(64)}`,
              totalTickets: 0n, winningTicket: 0n, winner: A(0), uri: "", seedRound: carried ? this.draws[carried - 1].seedRound : roundAt(this.headTs) + 20n,
              announcedAt: this.headTs, announcedAtBlock: this.evm(), snapshotAt: 0 });
            this.inFlight.set(coin, this.draws.length);
          }
          return encodeAbiParameters([{ type: "uint256" }], [BigInt(this.draws.length - (apply ? 1 : 0))]);
        }
        case "openDraw": {
          onlyKeeper();
          const [id, root, total, uri, proof, sig] = d.args as [bigint, Hex, bigint, string, Hex[], Hex];
          const x = draw(id);
          if (x.stage !== 7) this.revert("WrongStage");
          if (passed(x.announcedAt + 6 * 3_600, x.announcedAtBlock + 1_800)) this.revert("WindowOver");
          if (root === `0x${"0".repeat(64)}`) this.revert("BadInput");
          const at = this.snapshotTimeOf(Number(id), this.proven(x.seedRound, sig));
          // fixes-R3-carry: an empty snapshot's tree is its sentinel alone
          if (total === 0n ? proof.length !== 0 || root.toLowerCase() !== ticketLeaf(A(0), 0n, BigInt(at)).toLowerCase() : !verify(proof, root, ticketLeaf(A(0), total, BigInt(at)))) this.revert("BadProof");
          if (apply) Object.assign(x, { stage: 1, root, totalTickets: total, uri, snapshotAt: at, challengeEnds: this.headTs + 900, challengeEndsBlock: this.evm() + 75 });
          return "0x";
        }
        case "requestDraw": {
          const x = draw(d.args[0] as bigint);
          if (x.stage !== 1) this.revert("WrongStage");
          if (!passed(x.challengeEnds, x.challengeEndsBlock)) this.revert("ChallengeWindow");
          if (this.keeperPaused) this.revert("KeeperPaused");
          if (apply) {
            this.inFlight.delete(x.coin.toLowerCase()); this.carried.delete(x.coin.toLowerCase());
            // fixes-R3-carry: an empty draw closes with no winner (Empty), its unit back to the coin
            if (x.totalTickets === 0n) { x.stage = 8; this.units.set(x.coin.toLowerCase(), (this.units.get(x.coin.toLowerCase()) ?? 0n) + E18); }
            else Object.assign(x, { stage: 2, round: roundAt(this.headTs) + 20n, pinnedAt: this.headTs, pinnedAtBlock: this.evm() });
          }
          return "0x";
        }
        case "resolve": {
          const [id, sig] = d.args as [bigint, Hex];
          const x = draw(id);
          if (x.stage !== 2) this.revert("WrongStage");
          const rnd = this.proven(x.round, sig);
          if (apply) Object.assign(x, { stage: 3, resolvedAt: this.headTs, resolvedAtBlock: this.evm(),
            winningTicket: BigInt(keccak256(encodeAbiParameters([{ type: "bytes32" }, { type: "address" }, { type: "uint256" }], [rnd, RAFFLE, id]))) % x.totalTickets });
          return "0x";
        }
        case "claim": {
          const [id, holder, first, end, proof] = d.args as [bigint, Hex, bigint, bigint, Hex[]];
          const x = draw(id);
          if (x.stage !== 3) this.revert("WrongStage");
          if (holder === A(0) || first > x.winningTicket || x.winningTicket >= end || end > x.totalTickets) this.revert("BadProof");
          if (!verify(proof, x.root, ticketLeaf(holder, first, end))) this.revert("BadProof");
          if (apply) { x.stage = 4; x.winner = holder; this.claimed.push({ id: Number(id), holder: holder.toLowerCase() }); }
          return "0x";
        }
        case "voidDraw": {
          const x = draw(d.args[0] as bigint);
          const [at, atB] = x.stage === 3 ? [x.resolvedAt + 7 * 86_400, x.resolvedAtBlock + 50_400] : x.stage === 2 ? [x.pinnedAt + 7 * 86_400, x.pinnedAtBlock + 50_400]
            : x.stage === 7 ? [x.announcedAt + 6 * 3_600, x.announcedAtBlock + 1_800] : this.revert("WrongStage");
          if (!passed(at, atB)) this.revert("TooEarly");
          if (apply) { if (x.stage === 7) { this.inFlight.delete(x.coin.toLowerCase()); this.carried.set(x.coin.toLowerCase(), this.seedOrigin(Number(d.args[0])) + 1); } x.stage = 6; this.units.set(x.coin.toLowerCase(), (this.units.get(x.coin.toLowerCase()) ?? 0n) + E18); }
          return "0x";
        }
      }
    }
    if (t === FACT.toLowerCase()) {
      const d = decodeFunctionData({ abi: FACTORY, data }) as unknown as { functionName: string; args: unknown[] };
      switch (d.functionName) {
        case "coinCount": return enc(FACTORY, "coinCount", BigInt(this.coins.length));
        case "coins": return enc(FACTORY, "coins", this.coins[Number(d.args[0])].coin);
        case "policyOf": return enc(FACTORY, "policyOf", this.coins.find((c) => c.coin.toLowerCase() === String(d.args[0]).toLowerCase())?.policy ?? 0);
        case "routerOf": return enc(FACTORY, "routerOf", this.coins.find((c) => c.coin.toLowerCase() === String(d.args[0]).toLowerCase())?.curve ?? A(0));
      }
    }
    throw Object.assign(new Error("no such contract"), { rpc: { code: -32000, message: `the fake has no contract at ${to}` } });
  }

  private block(n: number) {
    return { number: `0x${n.toString(16)}`, timestamp: `0x${this.tsOf(n).toString(16)}`, baseFeePerGas: `0x${this.baseFee.toString(16)}`, l1BlockNumber: `0x${this.evm(n).toString(16)}` };
  }

  async handle(method: string, params: unknown[]): Promise<unknown> {
    this.asked.push(method);
    if (this.throttle > 0) { this.throttle--; throw Object.assign(new Error("throttled"), { rpc: { code: 429, message: "Too Many Requests" } }); }
    switch (method) {
      case "eth_chainId": return `0x${this.chainId.toString(16)}`;
      case "eth_blockNumber": return `0x${this.head.toString(16)}`;
      case "eth_getBlockByNumber": { const p = params[0] as string; const n = p === "latest" ? this.head : Number(BigInt(p)); return n > this.head || n < 0 ? null : this.block(n); }
      case "eth_getBalance": return `0x${this.balance.toString(16)}`;
      case "eth_getCode": return this.keeperCode;
      case "eth_maxPriorityFeePerGas": return `0x${this.tip.toString(16)}`;
      case "eth_estimateGas": { const c = params[0] as { from: string; to: string; data: Hex }; this.exec(c.from, c.to, c.data, false); return `0x${this.estimate.toString(16)}`; }
      case "eth_call": { const c = params[0] as { from?: string; to: string; data: Hex }; return this.exec(c.from ?? A(0), c.to, c.data, false); }
      case "eth_getTransactionCount": {
        const pending = [...this.txs.values()].filter((t) => t.block === null && !t.replaced).length;
        return `0x${(params[1] === "pending" ? this.nonce + pending : this.nonce).toString(16)}`;
      }
      case "eth_sendRawTransaction": {
        if (this.rejectSend) throw Object.assign(new Error(this.rejectSend), { rpc: { code: -32000, message: this.rejectSend } });
        const raw = params[0] as Hex;
        const hash = keccak256(raw);
        if (this.txs.has(hash)) throw Object.assign(new Error("already known"), { rpc: { code: -32000, message: "already known" } });
        const tx = parseTransaction(raw);
        const from = await recoverTransactionAddress({ serializedTransaction: raw as never });
        if (tx.chainId !== this.chainId) throw Object.assign(new Error("invalid chain id"), { rpc: { code: -32000, message: "invalid chain id for signer" } });
        if ((tx.nonce ?? 0) < this.nonce) throw Object.assign(new Error("nonce too low"), { rpc: { code: -32000, message: "nonce too low" } });
        if (tx.maxFeePerGas! < this.baseFee) throw Object.assign(new Error("fee"), { rpc: { code: -32000, message: "max fee per gas less than block base fee" } });
        const same = [...this.txs.values()].find((t) => t.block === null && !t.replaced && t.nonce === (tx.nonce ?? 0) && t.from.toLowerCase() === from.toLowerCase());
        if (same) {
          if (tx.maxFeePerGas! * 10n < same.maxFeePerGas * 11n) throw Object.assign(new Error("underpriced"), { rpc: { code: -32000, message: "replacement transaction underpriced" } });
          same.replaced = true;
        }
        this.sent.push(raw);
        const t: Tx = { hash, from, to: tx.to!, data: tx.data ?? "0x", nonce: tx.nonce ?? 0, gas: tx.gas!, maxFeePerGas: tx.maxFeePerGas!, block: null, status: 1 };
        this.txs.set(hash, t);
        if (this.mine) this.mineTx(t);
        return hash;
      }
      case "eth_getTransactionReceipt": {
        const t = this.txs.get(params[0] as string);
        if (!t || t.block === null) return null;
        return { status: t.status ? "0x1" : "0x0", gasUsed: "0x" + (60_000).toString(16), effectiveGasPrice: `0x${this.baseFee.toString(16)}`, blockNumber: `0x${t.block.toString(16)}`, transactionHash: t.hash };
      }
      case "eth_getLogs": return this.logs(params[0] as { address: string; topics: string[][] | string[]; fromBlock: string; toBlock: string });
    }
    throw Object.assign(new Error("method not found"), { rpc: { code: -32601, message: `the method ${method} does not exist` } });
  }

  mineTx(t: Tx): void {
    this.head += 1; this.headTs += 1;
    try { this.exec(t.from, t.to, t.data, true); t.status = 1; } catch { t.status = 0; }
    t.block = this.head; this.nonce = Math.max(this.nonce, t.nonce + 1);
    for (const o of this.txs.values()) if (o !== t && o.block === null && o.nonce === t.nonce && o.from.toLowerCase() === t.from.toLowerCase()) o.replaced = true;
  }

  private logs(q: { address: string; topics: (string | string[])[]; fromBlock: string; toBlock: string }): unknown[] {
    const from = Number(BigInt(q.fromBlock)), to = Number(BigInt(q.toBlock));
    if (this.logsLimitBlocks !== null && to - from + 1 > this.logsLimitBlocks) throw Object.assign(new Error("limit"), { rpc: { code: -32000, message: "logs matched by query exceeds limit of 10000" } });
    const coins = new Set([COIN.toLowerCase(), ...this.coins.map((c) => c.coin.toLowerCase())]);
    if (!coins.has(q.address.toLowerCase())) return [];
    // Transfer logs: every coin of the fake shares one transfer history (the tests use one coin at a time)
    return this.transfers.map((x, i) => ({ x, i })).filter(({ x }) => x.block >= from && x.block <= to).map(({ x, i }) => ({
      address: q.address, blockNumber: `0x${x.block.toString(16)}`, logIndex: `0x${i.toString(16)}`,
      topics: ["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef", `0x${x.from.slice(2).padStart(64, "0")}`, `0x${x.to.slice(2).padStart(64, "0")}`],
      data: `0x${x.value.toString(16).padStart(64, "0")}` }));
  }

  /** A drand relay: `<base>/<chain>/public/<round>`, served once the chain's clock has reached the round. */
  private relay(url: string): { status: number; body?: unknown } {
    const m = /^https:\/\/([^/]+)\/[0-9a-f]{64}\/public\/(\d+)$/.exec(url);
    if (!m) return { status: 404 };
    if (this.relayDown.has(m[1])) return { status: 503 };
    const r = BigInt(m[2]);
    if (timeOf(r) > this.headTs) return { status: 425 };
    const sig = this.relayLie && m[1] === "drand.cloudflare.com" ? sigOf(r + 1n) : sigOf(r);
    return { status: 200, body: { round: Number(r), signature: sig.slice(2), randomness: randomnessOf(sig).slice(2) } };
  }

  /** The node as a fetch: a thrown `{ rpc }` becomes a JSON-RPC error answer; `down` makes it a network failure. */
  down = false;
  fetch = (async (url: string, init: RequestInit = {}) => {
    const reply = (status: number, body: unknown, text?: string) => ({ ok: status >= 200 && status < 300, status, headers: new Headers(),
      json: async () => body, text: async () => text ?? JSON.stringify(body) });
    if (/\/public\/\d+$/.test(url)) { const r = this.relay(url); return reply(r.status, r.body); }
    if (/^https:\/\/(mirror|keeper|cdn)\.example\//.test(url)) {
      if ((init.method ?? "GET") === "PUT") {
        this.puts.push({ url, auth: (init.headers as Record<string, string>)?.authorization ?? null });
        this.files.set(url.replace("https://mirror.example/put/", "https://cdn.example/"), String(init.body));
        if (this.mirrorServesPut) this.files.set(url, String(init.body));       // the mirror's own object, as its API reads it back
        return reply(201, {});
      }
      const kept = this.files.get(url);
      if (kept !== undefined) return reply(200, JSON.parse(kept), kept);
      const m = /^https:\/\/keeper\.example\/snapshots\/(.+)$/.exec(url);
      if (m && this.serveDir && existsSync(join(this.serveDir, m[1]))) { const t = readFileSync(join(this.serveDir, m[1]), "utf8"); return reply(200, JSON.parse(t), t); }
      return reply(404, {});
    }
    if (this.down) throw new TypeError("fetch failed");
    const { id, method, params } = JSON.parse(String(init.body)) as { id: number; method: string; params: unknown[] };
    try { const result = await this.handle(method, params); return { ok: true, status: 200, json: async () => ({ jsonrpc: "2.0", id, result }) }; }
    catch (err) {
      const e = (err as { rpc?: { code: number; message: string; data?: unknown } }).rpc;
      if (!e) throw err;
      return { ok: true, status: 200, json: async () => ({ jsonrpc: "2.0", id, error: e }) };
    }
  }) as unknown as typeof fetch;
}

export const stackJson = (chainId = 4663) => JSON.stringify({ chainId, label: "stack", fromBlock: 10, stage: 0, registry: REG, owner: A(0xabc), keeper: A(0x5), treasury: TREAS,
  directory: A(0x100d), poolManager: PM, collections: [{ slug: "frogs", name: "Frogs", kind: "inscriptions", listingId: 2, wrapper: WRAP, factory: FACT, lock: LOCK, router: ROUTER,
    queue: MOD, floorModule: MOD, mintDesk: DESK, raffleVault: RAFFLE, burnVault: BURN, yoyoVault: YOYO, offerBoard: A(0), custody: { kind: "bitcoin", address: "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr" } }] });
