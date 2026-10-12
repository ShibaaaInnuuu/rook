import { readFileSync } from "node:fs";
import type { Deployment } from "../src/follow.js";
import { RpcError, type Rpc } from "../src/rpc.js";

export type Tape = { deployment: Deployment; tape: Record<string, unknown> };
export const loadTape = (): Tape => JSON.parse(readFileSync(new URL("./fixtures/e2e-rpc.json", import.meta.url), "utf8")) as Tape;

type Q = { address?: string | string[]; topics?: (string | string[] | null)[]; fromBlock: string; toBlock: string };
/** Every log the recording holds, wherever it holds it: the log database an eth_getLogs the recording was not asked for is answered from. */
const allLogs = (t: Tape): { address: string; topics: string[]; blockNumber: string }[] =>
  Object.entries(t.tape).filter(([k]) => k.startsWith("eth_getLogs|")).flatMap(([, v]) => v as { address: string; topics: string[]; blockNumber: string }[]);

/**
 * docs/phase10.md: the recording is a cache of exact questions, but the follower's questions changed when it learnt to ask
 * about curves and pools. An eth_getLogs the recording does not hold verbatim is answered from every log it does hold, filtered
 * the way a node would: by address, by each topic position, and by the block range. Any OTHER method it does not hold is still
 * a test failure rather than a network call.
 */
const matches = (l: { address: string; topics: string[]; blockNumber: string }, q: Q): boolean => {
  const want = q.address === undefined ? null : (Array.isArray(q.address) ? q.address : [q.address]).map((a) => a.toLowerCase());
  if (want && !want.includes(l.address.toLowerCase())) return false;
  for (const [i, t] of (q.topics ?? []).entries()) {
    if (t === null || t === undefined) continue;
    const any = (Array.isArray(t) ? t : [t]).map((x) => x.toLowerCase());
    if (!l.topics[i] || !any.includes(l.topics[i].toLowerCase())) return false;
  }
  const b = BigInt(l.blockNumber);
  return b >= BigInt(q.fromBlock) && b <= BigInt(q.toBlock);
};

/** Every recorded log a node would answer this query with. */
export const logsOf = (t: Tape, q: Q): { address: string; topics: string[]; blockNumber: string }[] => allLogs(t).filter((l) => matches(l, q));

/** Answers from the recording, and nothing else: a request the recording does not hold is a test failure, not a network call. */
export function replay(t: Tape, over: (method: string, params: unknown[]) => unknown | undefined = () => undefined): Rpc & { calls: string[] } {
  const calls: string[] = [];
  const fn = (async <T>(method: string, params: unknown[]): Promise<T> => {
    calls.push(method);
    const o = over(method, params);
    if (o !== undefined) return o as T;
    const k = `${method}|${JSON.stringify(params)}`;
    if (k in t.tape) return t.tape[k] as T;
    if (method === "eth_getLogs") return logsOf(t, params[0] as Q) as T;
    throw new RpcError("rpc", `not in the recording: ${k.slice(0, 160)}`);
  }) as Rpc & { calls: string[] };
  fn.calls = calls;
  return fn;
}
/** the recorded head's height, and the time of the newest recorded block (the clock the views should use) */
export const headOf = (t: Tape): number => Number(BigInt(t.tape["eth_blockNumber|[]"] as string));
export const clockOf = (t: Tape): number => Math.max(...Object.entries(t.tape).filter(([k]) => k.startsWith("eth_getBlockByNumber|")).map(([, v]) => Number(BigInt((v as { timestamp: string }).timestamp))));
