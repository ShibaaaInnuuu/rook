import { decodeFunctionResult, encodeFunctionData, type Hex } from "viem";
import { BEACON } from "./abis.js";
import { RpcAnswer, type ChainRpc, type Fetch, type Log } from "./rpc.js";
import { revertReason } from "./sender.js";

/**
 * fixes-B-C-keeper-draws.md §5 items 1–2. A drand evmnet round's signature, from public relays that must AGREE, then checked by
 * the chain's own beacon (`verify`, an eth_call: nothing is sent). BLS signatures are unique per round, so two honest relays give
 * the same bytes. A relay that answers differently is not a veto (audit R2-S2): the beacon is asked about each candidate, the one
 * it accepts is used, and the relay that served the other bytes is an alert (`drand-liar`). A relay that does not have the round
 * yet (404, 425, a future round) is "not yet". The beacon checks the signature on chain anyway: the relays are for liveness and a second
 * opinion, not trust.
 */
export const EVMNET_CHAIN = "04f1e9062b8a81f848fded9c12306733282b2727ecced50032187751166ec8c3";
/** Measured 2026-09-26: all four serve evmnet (round 1000 identical). Cloudflare is operated apart from the League's relays. */
export const DEFAULT_DRAND_URLS = ["https://api.drand.sh", "https://drand.cloudflare.com", "https://api2.drand.sh"] as const;

export class RelayDisagreement extends Error { constructor(message: string) { super(message); this.name = "RelayDisagreement"; } }

export type Signed = { round: bigint; signature: Hex; randomness: Hex; relays: string[]; liars: string[] };

export class DrandRelays {
  constructor(private readonly urls: readonly string[], private readonly fetchImpl: Fetch = fetch, private readonly log: Log = () => {},
    private readonly need = 2, private readonly timeoutMs = 10_000) {
    if (urls.length < need) throw new Error(`drand: ${need} relays must agree, and ${urls.length} are configured`);
  }

  private async one(base: string, round: bigint): Promise<Hex | null> {
    const host = (() => { try { return new URL(base).host; } catch { return "invalid-url"; } })();
    let r: Response;
    try {
      r = await this.fetchImpl(`${base.replace(/\/$/, "")}/${EVMNET_CHAIN}/public/${round}`, { headers: { accept: "application/json", "user-agent": "rook-keeper/1" },
        signal: AbortSignal.timeout(this.timeoutMs) });
    } catch (err) { this.log("warn", "drand-unreachable", { host, round: String(round), error: String((err as Error).message).slice(0, 120) }); return null; }
    if (r.status === 404 || r.status === 425 || r.status === 400) return null;                  // not produced yet
    if (!r.ok) { this.log("warn", "drand-http", { host, round: String(round), status: r.status }); return null; }
    let j: { round?: unknown; signature?: unknown };
    try { j = (await r.json()) as typeof j; } catch { return null; }
    if (String(j.round) !== String(round)) return null;
    const sig = String(j.signature ?? "");
    if (!/^[0-9a-f]{128}$/i.test(sig)) { this.log("warn", "drand-garbled", { host, round: String(round) }); return null; }
    return `0x${sig.toLowerCase()}` as Hex;
  }

  /**
   * Every distinct signature the relays serve for the round, each with the relays that served it (most-served first); null when
   * no relay has it yet. Audit R2-S2: a relay that answers other bytes is no longer a veto on its own; the caller lets the chain's
   * beacon say which candidate is the round's signature (BLS signatures are unique: at most one verifies).
   */
  async candidates(round: bigint): Promise<{ signature: Hex; relays: string[] }[] | null> {
    const got = await Promise.all(this.urls.map(async (u) => ({ u, sig: await this.one(u, round) })));
    const by = new Map<Hex, string[]>();
    for (const g of got) if (g.sig !== null) by.set(g.sig, [...(by.get(g.sig) ?? []), new URL(g.u).host]);
    if (!by.size) return null;
    return [...by].map(([signature, relays]) => ({ signature, relays })).sort((a, b) => b.relays.length - a.relays.length);
  }

  /**
   * The round's signature when at least `need` relays give the same bytes and no relay gives others; null when fewer have it yet.
   * Throws RelayDisagreement when two relays give different bytes (provenRound settles that case with the beacon instead).
   */
  async signature(round: bigint): Promise<{ signature: Hex; relays: string[] } | null> {
    const c = await this.candidates(round);
    if (!c) return null;
    if (c.length > 1) throw new RelayDisagreement(`drand relays disagree on round ${round}: ${c.map((x) => `${x.relays.join("+")}=${x.signature.slice(0, 18)}…`).join(", ")}`);
    return c[0].relays.length >= this.need ? c[0] : null;
  }

  get quorum(): number { return this.need; }
}

/**
 * A round's signature, served by at least `need` relays AND accepted by the chain's beacon (`verify(round, sig)`, which returns the
 * randomness, sha256 of the signature). Null: not available yet. Audit R2-S2: when relays serve different bytes, every candidate is
 * put to the beacon; the one it accepts is used (if enough relays serve it) and the relays that served anything else are named in
 * `liars` for the caller's alert. Throws only when the beacon accepts none of what the relays serve.
 */
export async function provenRound(relays: DrandRelays, rpc: ChainRpc, beacon: Hex, round: bigint): Promise<Signed | null> {
  const cands = await relays.candidates(round);
  if (!cands) return null;
  const refused: string[] = [];
  for (const c of cands) {
    let out: Hex;
    try {
      out = await rpc.call<Hex>("eth_call", [{ to: beacon, data: encodeFunctionData({ abi: BEACON, functionName: "verify", args: [round, c.signature] }) }, "latest"]);
    } catch (err) {
      if (err instanceof RpcAnswer) { refused.push(`${c.relays.join("+")}: ${revertReason(err)}`); continue; }
      throw err;
    }
    const liars = cands.filter((x) => x !== c).flatMap((x) => x.relays);
    if (c.relays.length < relays.quorum) {
      // the beacon's signature, but not yet served by enough relays: wait (the liars are still worth saying)
      if (liars.length) throw new RelayDisagreement(`drand round ${round}: ${liars.join(", ")} served bytes the beacon refuses; the right signature is served by ${c.relays.join(", ")} only, waiting for a second relay`);
      return null;
    }
    const randomness = decodeFunctionResult({ abi: BEACON, functionName: "verify", data: out }) as Hex;
    return { round, signature: c.signature, randomness, relays: c.relays, liars };
  }
  throw new RelayDisagreement(`the beacon refuses every signature the relays serve for round ${round}: ${refused.join("; ")}`);
}
