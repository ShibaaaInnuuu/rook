import { readFileSync } from "node:fs";
import { encodeFunctionData, parseAbi, type Hex } from "viem";
import { arm, NotArmed, type Deployment, type Role } from "./guard.js";

/**
 * docs/phase6.md, tier 2. The keeper's and the attestor's hands in a SMALL REAL TEST on Robinhood Chain. Run by the user, never by
 * anything in this repository. One command, one transaction, and nothing is signed unless keeper/live/guard.ts is armed on all
 * five facts. `--dry` prints the call (to, data) and signs nothing: paste it into a hardware wallet's tool instead, if you prefer.
 *
 *   npm run live -- post-cap <listing> <capWei>
 *   npm run live -- declare <listing> <maxEthWei> <quantity> <capRef>          refused above KEEPER_TEST_MAX_WEI (default 0.02 ETH)
 *   npm run live -- open-draw <listing> <acquisition> <snapshot.json> <uri>
 *   npm run live -- record-delivery <listing> <acquisition> <bitcoin txid>
 *   npm run live -- settle-inscription <listing> <purchase> <inscription id> <bitcoin txid> <sats> [proof.json]      (attestor)
 *   npm run live -- settle-rune <listing> <purchase> <raw amount> <bitcoin txid> <sats>                               (attestor)
 * Environment: RPC_URL, DEPLOYMENT (default ../deployments/test-4663.json), and for the role that signs:
 *   <ROLE>_MODE=mainnet-test  <ROLE>_LIVE=1  <ROLE>_CONFIRM=<role>-live:4663:<address>  <ROLE>_KEY=<set by you, never by us>
 */
const REG = parseAbi(["function vaultOf(uint256 id) view returns (address)"]);
const VAULT = parseAbi(["function postCap(uint256 capWei)", "function declarePurchase(uint256 maxEth, uint256 quantity, string capRef) returns (uint256 id)",
  "function settleInscription(uint256 id, string inscriptionId, bytes32 btcTxid, uint64 satsPaid, bytes32[] proof) returns (uint256)",
  "function settleRune(uint256 id, uint256 amountReceived, bytes32 btcTxid, uint64 satsPaid)"]);
const DIST = parseAbi(["function openDraw(uint256 listingId, uint256 acquisitionId, bytes32 root, uint256 totalTickets, string uri)",
  "function recordDelivery(uint256 listingId, uint256 acquisitionId, bytes32 txid)"]);
export const TEST_MAX_WEI_DEFAULT = 2n * 10n ** 16n;

export type Built = { role: Role; to: "vault" | "distributor"; listing: bigint; data: Hex; what: string };
export function build(argv: readonly string[], env: Readonly<Record<string, string | undefined>> = {}): Built {
  const [cmd, ...a] = argv;
  const n = (v: string | undefined, name: string) => { if (v === undefined || !/^\d+$/.test(v)) throw new Error(`${name} must be a whole number`); return BigInt(v); };
  const txid = (v: string | undefined) => { if (!/^[0-9a-fA-F]{64}$/.test(v ?? "")) throw new Error("a Bitcoin txid is 64 hex characters"); return `0x${v}` as Hex; };
  switch (cmd) {
    case "post-cap": return { role: "keeper", to: "vault", listing: n(a[0], "listing"), data: encodeFunctionData({ abi: VAULT, functionName: "postCap", args: [n(a[1], "capWei")] }), what: `post a cap of ${a[1]} wei` };
    case "declare": {
      const max = n(a[1], "maxEthWei"), ceiling = BigInt(env.KEEPER_TEST_MAX_WEI ?? TEST_MAX_WEI_DEFAULT);
      if (max > ceiling) throw new Error(`refused: ${max} wei is over this test's ceiling of ${ceiling} wei (KEEPER_TEST_MAX_WEI). A small real test stays small.`);
      if (!a[3]) throw new Error("capRef is required: where the price came from");
      return { role: "keeper", to: "vault", listing: n(a[0], "listing"), data: encodeFunctionData({ abi: VAULT, functionName: "declarePurchase", args: [max, n(a[2], "quantity"), a[3]] }), what: `declare a purchase of up to ${max} wei for ${a[2]}` };
    }
    case "open-draw": {
      const s = JSON.parse(readFileSync(a[2], "utf8")) as { root: Hex; totalTickets?: string };
      if (!s.totalTickets || !a[3]) throw new Error("a draw needs a ticket snapshot (weighted or per-wallet) and the URI it is published at");
      return { role: "keeper", to: "distributor", listing: n(a[0], "listing"), data: encodeFunctionData({ abi: DIST, functionName: "openDraw", args: [n(a[0], "listing"), n(a[1], "acquisition"), s.root, BigInt(s.totalTickets), a[3]] }), what: `open a draw with root ${s.root}` };
    }
    case "record-delivery": return { role: "keeper", to: "distributor", listing: n(a[0], "listing"), data: encodeFunctionData({ abi: DIST, functionName: "recordDelivery", args: [n(a[0], "listing"), n(a[1], "acquisition"), txid(a[2])] }), what: `record delivery ${a[2]}` };
    case "settle-inscription": return { role: "attestor", to: "vault", listing: n(a[0], "listing"), data: encodeFunctionData({ abi: VAULT, functionName: "settleInscription",
      args: [n(a[1], "purchase"), a[2] ?? "", txid(a[3]), n(a[4], "sats"), a[5] ? (JSON.parse(readFileSync(a[5], "utf8")) as Hex[]) : []] }), what: `settle purchase ${a[1]} with inscription ${a[2]}` };
    case "settle-rune": return { role: "attestor", to: "vault", listing: n(a[0], "listing"), data: encodeFunctionData({ abi: VAULT, functionName: "settleRune", args: [n(a[1], "purchase"), n(a[2], "amount"), txid(a[3]), n(a[4], "sats")] }), what: `settle purchase ${a[1]} with ${a[2]} raw rune units` };
    default: throw new Error("commands: post-cap, declare, open-draw, record-delivery, settle-inscription, settle-rune");
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const argv = process.argv.slice(2), dry = argv.includes("--dry");
  try {
    const b = build(argv.filter((x) => x !== "--dry"), process.env);
    const url = process.env.RPC_URL ?? "https://rpc.mainnet.chain.robinhood.com";
    const dep = JSON.parse(readFileSync(process.env.DEPLOYMENT ?? new URL("../../deployments/test-4663.json", import.meta.url).pathname, "utf8")) as Deployment & Record<string, string>;
    let to = dep.distributor;
    if (b.to === "vault") {
      const r = await fetch(url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "eth_call",
        params: [{ to: dep.registry, data: encodeFunctionData({ abi: REG, functionName: "vaultOf", args: [b.listing] }) }, "latest"] }) });
      to = `0x${((await r.json()) as { result: string }).result.slice(-40)}`;
    }
    if (dry) { console.log(JSON.stringify({ dry: true, role: b.role, what: b.what, to, data: b.data })); process.exit(0); }
    const armed = await arm(b.role, url, dep);
    console.log(JSON.stringify({ role: b.role, signer: armed.address, what: b.what, to }));
    console.log(JSON.stringify({ sent: await armed.send({ to, data: b.data }) }));
  } catch (err) {
    if (err instanceof NotArmed) { console.error(err.message); process.exit(3); }
    console.error(String((err as Error).message)); process.exit(2);
  }
}
