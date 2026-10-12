import { readFileSync } from "node:fs";
import { encodeFunctionData, parseAbi, type Hex } from "viem";
import { stagingSender } from "../../keeper/staging/guard.js";
import { STAGING_MEMBERS, memberId, proofOf } from "../../keeper/staging/members.js";

/**
 * docs/phase6.md: record ONE REAL purchase on staging, by hand. The operator settles released purchases with fixtures; start the
 * chain with STAGING_MANUAL_SETTLE=1 and it leaves them for this. Run it where the node is (`railway ssh --service staging-chain`):
 *   npm run attest -- <listing id> <purchase id> <bitcoin txid> <inscription id>            an inscription you bought yourself
 *   npm run attest -- <listing id> <purchase id> <bitcoin txid> <rune id> <raw amount>      runes you bought yourself
 * It sends as the staging attestor through the node on this machine (the staging chain only). The txid is recorded as given: nothing on
 * this chain can check it, which is exactly the trust the real thing places in its attestor. Anyone can check it on Bitcoin.
 */
const [listing, purchase, txid, ref, amount] = process.argv.slice(2);
if (!listing || !purchase || !/^[0-9a-fA-F]{64}$/.test(txid ?? "") || !ref) { console.error("usage: npm run attest -- <listing> <purchase> <64-hex bitcoin txid> <member:N | rune id> [raw rune amount]   (member:N is fixture member N of the staging collection, 0 to 1023, not yet held; SATS_PAID=<sats>)"); process.exit(2); }
const NODE = process.env.STAGING_NODE ?? "http://127.0.0.1:8546", DATA = process.env.STAGING_DATA ?? "/data";
const d = JSON.parse(readFileSync(`${DATA}/deployment.json`, "utf8")) as Record<string, string>;
const s = await stagingSender(NODE);
const REG = parseAbi(["function vaultOf(uint256 id) view returns (address)"]);
const VAULT = parseAbi(["function settleInscription(uint256 id, string inscriptionId, bytes32 btcTxid, uint64 satsPaid, bytes32[] proof) returns (uint256)",
  "function settleRune(uint256 id, uint256 amountReceived, bytes32 btcTxid, uint64 satsPaid)"]);
const vault = `0x${(await s.request<Hex>("eth_call", [{ to: d.registry, data: encodeFunctionData({ abi: REG, functionName: "vaultOf", args: [BigInt(listing)] }) }, "latest"])).slice(-40)}`;
// the staging collection's members are fixtures (keeper/staging/members.ts): the TXID is what is real here, the inscription id is not
const member = /^member:\d+$/.test(ref) && Number(ref.slice(7)) < STAGING_MEMBERS ? Number(ref.slice(7)) : null;
const sats = BigInt(process.env.SATS_PAID ?? "0");
const data = amount === undefined
  ? encodeFunctionData({ abi: VAULT, functionName: "settleInscription", args: [BigInt(purchase), member === null ? ref : memberId(member), `0x${txid}`, sats,
      member === null ? (JSON.parse(process.env.MEMBERSHIP_PROOF ?? "[]") as Hex[]) : proofOf(member)] })
  : encodeFunctionData({ abi: VAULT, functionName: "settleRune", args: [BigInt(purchase), BigInt(amount), `0x${txid}`, sats] });
const r = await s.send({ from: d.attestor, to: vault, data });
console.log(JSON.stringify({ settled: true, listing, purchase, txid, ref, amount: amount ?? null, tx: r.transactionHash, note: "a REAL receipt recorded by hand on staging; check the txid on any Bitcoin explorer" }));
