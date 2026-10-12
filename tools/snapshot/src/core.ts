import { encodeAbiParameters, keccak256, concat, type Hex } from "viem";

/** docs/snapshot-v1.md. Everything here is pure: logs in, a root and proofs out. No network, no clock. */
export type TransferLog = { blockNumber: bigint; logIndex: number; from: string; to: string; value: bigint };
export type Kind = "weighted" | "per-wallet" | "pro-rata";
export type Entry = { holder: string; balance: string; first?: string; end?: string; units?: string; leaf: Hex; proof: Hex[] };

const lc = (a: string) => a.toLowerCase();
const byAddress = (a: string, b: string) => (BigInt(a) < BigInt(b) ? -1 : BigInt(a) > BigInt(b) ? 1 : 0);

/** Balances at the end of the last log given, from Transfer logs in (block, log index) order. Throws if a balance would go negative. */
export function balancesOf(logs: readonly TransferLog[]): Map<string, bigint> {
  const sorted = [...logs].sort((x, y) => (x.blockNumber === y.blockNumber ? x.logIndex - y.logIndex : x.blockNumber < y.blockNumber ? -1 : 1));
  const bal = new Map<string, bigint>();
  const ZERO = "0x0000000000000000000000000000000000000000";
  for (const l of sorted) {
    const from = lc(l.from), to = lc(l.to);
    if (from !== ZERO) {
      const next = (bal.get(from) ?? 0n) - l.value;
      if (next < 0n) throw new Error(`logs are missing: ${from} would go negative at block ${l.blockNumber}, log ${l.logIndex}`);
      bal.set(from, next);
    }
    if (to !== ZERO) bal.set(to, (bal.get(to) ?? 0n) + l.value);
  }
  return bal;
}

export const ticketLeaf = (holder: string, first: bigint, end: bigint): Hex =>
  keccak256(keccak256(encodeAbiParameters([{ type: "address" }, { type: "uint256" }, { type: "uint256" }], [holder as Hex, first, end])));
export const shareLeaf = (holder: string, units: bigint): Hex =>
  keccak256(keccak256(encodeAbiParameters([{ type: "address" }, { type: "uint256" }], [holder as Hex, units])));

const pairHash = (a: Hex, b: Hex): Hex => (BigInt(a) <= BigInt(b) ? keccak256(concat([a, b])) : keccak256(concat([b, a])));

/** Sorted leaves, neighbours paired left to right with the commutative hash, an odd node moves up unchanged. */
export function merkle(leaves: readonly Hex[]): { root: Hex; proofOf: (leaf: Hex) => Hex[] } {
  if (leaves.length === 0) throw new Error("an empty snapshot has no root");
  const sorted = [...leaves].sort((a, b) => (BigInt(a) < BigInt(b) ? -1 : BigInt(a) > BigInt(b) ? 1 : 0));
  for (let i = 1; i < sorted.length; i++) if (sorted[i] === sorted[i - 1]) throw new Error("duplicate leaf");
  const layers: Hex[][] = [sorted];
  while (layers[layers.length - 1].length > 1) {
    const prev = layers[layers.length - 1], next: Hex[] = [];
    for (let i = 0; i < prev.length; i += 2) next.push(i + 1 < prev.length ? pairHash(prev[i], prev[i + 1]) : prev[i]);
    layers.push(next);
  }
  return {
    root: layers[layers.length - 1][0],
    proofOf: (leaf) => {
      let idx = sorted.indexOf(leaf);
      if (idx < 0) throw new Error("not a leaf of this tree");
      const proof: Hex[] = [];
      for (let d = 0; d < layers.length - 1; d++) {
        const layer = layers[d], sib = idx ^ 1;
        if (sib < layer.length) proof.push(layer[sib]);
        idx >>= 1;
      }
      return proof;
    },
  };
}

/** What OpenZeppelin's MerkleProof.verify computes; here so a third party can check a proof without a chain. */
export const verify = (proof: readonly Hex[], root: Hex, leaf: Hex): boolean => proof.reduce((h, p) => pairHash(h, p), leaf) === root;

/**
 * `snapshotAt` (v2, docs/snapshot-v1.md "Version 2"): a SweepVault draw's snapshot time. The tree then also holds the sentinel leaf
 * `leaf(address(0), totalTickets, snapshotAt)`, which commits the ticket count and the time to the root (`openDraw` requires its
 * proof; `claim` refuses holder zero). Weighted only. Without it the output is v1, byte for byte. With no eligible holder the tree
 * is the sentinel alone: root = `sentinelLeaf(0, snapshotAt)`, proof `[]`, no entries; that is exactly what `openDraw` takes for
 * an empty draw, which then closes with no winner (fixes-R3-carry).
 */
export type Inputs = { coin: string; block: string; kind: Kind; decimals: number; minBalance?: string; total?: string; exclusions: string[]; snapshotAt?: string };
export type Sentinel = { holder: "0x0000000000000000000000000000000000000000"; first: string; end: string; leaf: Hex; proof: Hex[] };
export type Snapshot = { version: 1 | 2; inputs: Inputs; holders: number; root: Hex; totalTickets?: string; allocated?: string; remainder?: string;
  sentinel?: Sentinel; entries: Entry[] };

export const ZERO_ADDRESS = "0x0000000000000000000000000000000000000000" as const;
/** The v2 sentinel leaf: the usual ticket leaf of holder zero over the range [totalTickets, snapshotAt). */
export const sentinelLeaf = (totalTickets: bigint, snapshotAt: bigint): Hex => ticketLeaf(ZERO_ADDRESS, totalTickets, snapshotAt);

export function snapshot(balances: ReadonlyMap<string, bigint>, inputs: Inputs): Snapshot {
  const excluded = new Set(inputs.exclusions.map(lc));
  const holders = [...balances].filter(([a, b]) => b > 0n && !excluded.has(lc(a))).map(([a, b]) => [lc(a), b] as const).sort((x, y) => byAddress(x[0], y[0]));
  const normal: Inputs = { ...inputs, coin: lc(inputs.coin), exclusions: [...excluded].sort(byAddress) };
  const rows: { holder: string; balance: bigint; first?: bigint; end?: bigint; units?: bigint; leaf: Hex }[] = [];
  let extra: Partial<Snapshot> = {};
  if (inputs.kind === "pro-rata") {
    const total = BigInt(inputs.total ?? "0");
    if (total <= 0n) throw new Error("pro-rata needs the total units being distributed");
    const sum = holders.reduce((s, [, b]) => s + b, 0n);
    let allocated = 0n;
    for (const [holder, balance] of holders) {
      const units = sum === 0n ? 0n : (total * balance) / sum;
      if (units === 0n) continue;
      allocated += units;
      rows.push({ holder, balance, units, leaf: shareLeaf(holder, units) });
    }
    extra = { allocated: String(allocated), remainder: String(total - allocated) };
  } else {
    const one = 10n ** BigInt(inputs.decimals), min = BigInt(inputs.minBalance ?? String(one));
    let next = 0n;
    for (const [holder, balance] of holders) {
      const n = inputs.kind === "weighted" ? balance / one : balance >= min ? 1n : 0n;
      if (n === 0n) continue;
      rows.push({ holder, balance, first: next, end: next + n, leaf: ticketLeaf(holder, next, next + n) });
      next += n;
    }
    extra = { totalTickets: String(next) };
  }
  let sentinel: Hex | null = null;
  if (inputs.snapshotAt !== undefined) {
    if (inputs.kind !== "weighted") throw new Error("a snapshot time (the v2 sentinel) is for weighted SweepVault draws only");
    if (!/^\d{1,20}$/.test(inputs.snapshotAt)) throw new Error("snapshotAt is a unix time in seconds");
    sentinel = sentinelLeaf(BigInt(extra.totalTickets as string), BigInt(inputs.snapshotAt));
  }
  const tree = merkle([...rows.map((r) => r.leaf), ...(sentinel ? [sentinel] : [])]);
  const entries: Entry[] = rows.map((r) => ({ holder: r.holder, balance: String(r.balance),
    ...(r.units !== undefined ? { units: String(r.units) } : { first: String(r.first), end: String(r.end) }), leaf: r.leaf, proof: tree.proofOf(r.leaf) }));
  if (!sentinel) return { version: 1, inputs: normal, holders: rows.length, root: tree.root, ...extra, entries };
  return { version: 2, inputs: normal, holders: rows.length, root: tree.root, ...extra,
    sentinel: { holder: ZERO_ADDRESS, first: extra.totalTickets as string, end: inputs.snapshotAt!, leaf: sentinel, proof: tree.proofOf(sentinel) }, entries };
}

/** Fixed key order, no timestamps, a trailing newline: the same inputs give the same bytes. */
export const serialise = (s: Snapshot): string => JSON.stringify(s, null, 2) + "\n";
