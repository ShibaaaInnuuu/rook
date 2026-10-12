import { decodeFunctionResult, encodeFunctionData, parseAbi, type Abi, type Hex } from "viem";

/** docs/keeper-dryrun-v1.md. One JSON-RPC function, and it refuses anything that is not a read. */
export const READ_ONLY = ["eth_call", "eth_getLogs", "eth_blockNumber", "eth_chainId", "eth_getBlockByNumber"] as const;
export type Fetch = typeof fetch;

export async function rpc<T>(url: string, method: string, params: unknown[], fetchImpl: Fetch = fetch): Promise<T> {
  if (!(READ_ONLY as readonly string[]).includes(method)) throw new Error(`refused: ${method} is not a read, and this package only reads`);
  const r = await fetchImpl(url, { method: "POST", headers: { "content-type": "application/json", "user-agent": "etch-keeper-dryrun/1" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) });
  const j = (await r.json()) as { result?: T; error?: { message: string } };
  if (j.error) throw new Error(`${method}: ${j.error.message}`);
  return j.result as T;
}

const REGISTRY: Abi = parseAbi([
  "function count() view returns (uint256)",
  "function listing(uint256 id) view returns ((uint8 kind, bool paused, uint8 divisibility, address vault, bytes32 membershipRoot, string name, string ref, string btcVault))",
]);
const VAULT: Abi = parseAbi([
  "function attributed() view returns (uint256)", "function reserved() view returns (uint256)", "function capWei() view returns (uint256)",
  "function capExpiry() view returns (uint64)", "function isBlocked() view returns (bool, uint256)", "function openPurchases() view returns (uint256[])",
]);

export type ListingState = { id: number; kind: "inscriptions" | "rune"; name: string; ref: string; divisibility: number; vault: string; paused: boolean;
  spendableWei: bigint; capWei: bigint; capExpiry: number; open: number; blockedBy: number | null };

/**
 * The state of every listing, or — given `ids` — only of those. docs/phase11.md §7: with 58 listings a full pass is seven reads
 * each, every tick, for ever; the staging operator passes the ones it actually has work for plus a rotating window, and a
 * listing's identity (kind, ref, divisibility, vault) never changes once it is listed.
 */
export async function readListings(url: string, registry: string, fetchImpl: Fetch = fetch, ids?: readonly number[]): Promise<ListingState[]> {
  const read = async (to: string, abi: Abi, functionName: string, args: unknown[] = []) => {
    const data = encodeFunctionData({ abi, functionName, args });
    const out = await rpc<Hex>(url, "eth_call", [{ to, data }, "latest"], fetchImpl);
    return decodeFunctionResult({ abi, functionName, data: out }) as unknown;
  };
  const n = Number(await read(registry, REGISTRY, "count"));
  const out: ListingState[] = [];
  for (const id of ids ?? Array.from({ length: n }, (_x, i) => i)) {
    if (id < 0 || id >= n) continue;
    const l = (await read(registry, REGISTRY, "listing", [BigInt(id)])) as { kind: number; paused: boolean; divisibility: number; vault: string; name: string; ref: string };
    const [attributed, reserved, capWei, capExpiry, blocked, open] = await Promise.all([read(l.vault, VAULT, "attributed"), read(l.vault, VAULT, "reserved"),
      read(l.vault, VAULT, "capWei"), read(l.vault, VAULT, "capExpiry"), read(l.vault, VAULT, "isBlocked"), read(l.vault, VAULT, "openPurchases")]);
    const [isBlocked, by] = blocked as [boolean, bigint];
    out.push({ id, kind: l.kind === 0 ? "inscriptions" : "rune", name: l.name, ref: l.ref, divisibility: l.divisibility, vault: l.vault, paused: l.paused,
      spendableWei: (attributed as bigint) - (reserved as bigint), capWei: capWei as bigint, capExpiry: Number(capExpiry), open: (open as bigint[]).length,
      blockedBy: isBlocked ? Number(by) : null });
  }
  return out;
}
