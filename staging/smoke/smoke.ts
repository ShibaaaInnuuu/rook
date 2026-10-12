import { createPublicClient, createWalletClient, decodeEventLog, defineChain, http, keccak256, parseAbi, toHex, type Hex } from "viem";
import { generatePrivateKey, privateKeyToAccount } from "viem/accounts";
import { balancesOf, snapshot } from "../../tools/snapshot/src/core.js";
import { transferLogs } from "../../tools/snapshot/src/rpc.js";
import { readFileSync } from "node:fs";
import { STAGING_CHAIN_ID, STAGING_NAME, STAGING_CURRENCY } from "../chain-id.js";

/**
 * docs/phase6.md §6. The whole cycle as STRANGERS would do it: through the public proxy, with wallets that sign their own
 * transactions for the staging chain (staging/chain-id.json), while the staging operator does the keeper's, purchaser's and attestor's parts beside the node.
 *   npm run smoke -- http://127.0.0.1:8650 [http://127.0.0.1:8787]
 * The keys here are made in this process, fund themselves from the faucet, and are never written anywhere. The script refuses any
 * chain but that one: a throwaway key must never sign for a chain where a signature is worth something.
 */
const RPC = process.argv[2] ?? "http://127.0.0.1:8650", API = process.argv[3] ?? null;
const TIMEOUT_S = Number(process.env.SMOKE_TIMEOUT_S ?? 600);
const steps: string[] = [];
const ok = (s: string, o: Record<string, unknown> = {}) => { steps.push(s); console.log(`ok  ${s}${Object.keys(o).length ? "  " + JSON.stringify(o, (_k, v) => (typeof v === "bigint" ? String(v) : v)) : ""}`); };
const fail = (s: string): never => { console.error(`FAIL ${s}`); process.exit(1); };
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
async function until<T>(what: string, f: () => Promise<T | null | undefined | false>, everyMs = 3_000): Promise<T> {
  const end = Date.now() + TIMEOUT_S * 1000;
  while (Date.now() < end) { const v = await f().catch(() => null); if (v) return v; await sleep(everyMs); }
  return fail(`timed out waiting for: ${what}`);
}

const info = (await (await fetch(`${RPC}/info`)).json()) as { chainId: number; deployment: Record<string, string> & { fromBlock: number } | null };
if (info.chainId !== STAGING_CHAIN_ID) fail(`this is chain ${info.chainId}; the smoke test signs for ${STAGING_CHAIN_ID} and nothing else`);
const d = info.deployment ?? fail("the staging chain has no deployment yet");
const chain = defineChain({ id: STAGING_CHAIN_ID, name: STAGING_NAME, nativeCurrency: STAGING_CURRENCY, rpcUrls: { default: { http: [RPC] } } });
const pub = createPublicClient({ chain, transport: http(RPC) });
if ((await pub.getChainId()) !== STAGING_CHAIN_ID) fail(`the node behind the proxy is not ${STAGING_CHAIN_ID}`);

// the proxy's refusals, from outside, first: if any of these is forwarded the rest of the test is on a chain anyone can rewrite
for (const method of ["anvil_setBalance", "evm_mine", "eth_sendTransaction", "eth_accounts", "anvil_impersonateAccount", "debug_traceTransaction", "hardhat_setCode"]) {
  const r = (await (await fetch(RPC, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params: [] }) })).json()) as { error?: { message: string } };
  if (!r.error || !/refused by the staging proxy/.test(r.error.message)) fail(`${method} was not refused by the proxy`);
}
ok("the proxy refuses every admin method and every unsigned send");

const LAUNCHER = parseAbi([
  "struct Socials { string twitter; string telegram; string discord; string website; string farcaster; }",
  "struct LaunchParams { string name; string symbol; string logo; string description; Socials socials; uint16 creatorTaxBps; bytes32 expectedEconomics; bytes32 salt; uint256 launchConfigId; uint256 listingId; }",
  "function launch(LaunchParams p) payable returns (address coin, address curve, address router)",
  "event Launched(address indexed coin, address indexed creator, uint256 indexed listingId, address curve, address router)",
]);
const FACTORY = parseAbi(["function launchFee() view returns (uint256)"]);
const CURVE = parseAbi(["function buy(uint256 quoteIn, uint256 minTokensOut, address recipient) payable returns (uint256)"]);
const ROUTER = parseAbi(["function harvest() returns (uint256 toVault, uint256 toTreasury)", "event Harvested(uint256 claimed, uint256 toVault, uint256 toTreasury)"]);
const REGISTRY = parseAbi(["function vaultOf(uint256 id) view returns (address)"]);
const VAULT = parseAbi(["function acquisitionCount() view returns (uint256)", "function purchaseCount() view returns (uint256)",
  "function acquisition(uint256 id) view returns ((uint256 purchaseId, address coin, uint256 amount, bytes32 btcTxid, uint64 satsPaid, string assetRef))"]);
const DIST = parseAbi([
  "function choose(uint256 listingId, uint256 acquisitionId, uint8 mode, address named)",
  "function setDestination(uint256 listingId, uint256 acquisitionId, string taprootAddress)",
  "function lot(uint256 listingId, uint256 acquisitionId) view returns ((uint8 mode, uint8 stage, bool overdue, uint64 challengeEnds, uint64 round, uint64 pinnedAt, uint64 destinationAt, address winner, bytes32 root, uint256 totalTickets, uint256 winningTicket, bytes32 deliveryTxid, string uri, string destination))",
]);

const people = Object.fromEntries(["creator", "alice", "bob", "carol"].map((n) => [n, privateKeyToAccount(generatePrivateKey())]));
const wallet = (n: string) => createWalletClient({ account: people[n], chain, transport: http(RPC) });
const mined = async (hash: Hex) => { const r = await pub.waitForTransactionReceipt({ hash, pollingInterval: 500 }); if (r.status !== "success") fail(`reverted: ${hash}`); return r; };
for (const n of Object.keys(people)) {
  const r = await fetch(`${RPC}/faucet`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ address: people[n].address }) });
  if (!r.ok) fail(`the faucet refused ${n}: ${await r.text()}`);
}
if ((await pub.getBalance({ address: people.alice.address })) === 0n) fail("the faucet gave nothing");
const again = await fetch(`${RPC}/faucet`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ address: people.alice.address }) });
if (again.status !== 429) fail("the faucet gave twice to one address in a day");
ok("four fresh wallets funded by the faucet; a second ask is refused");

const LISTING = 0n;
const vault = await pub.readContract({ address: d.registry as Hex, abi: REGISTRY, functionName: "vaultOf", args: [LISTING] });
const before = await pub.readContract({ address: vault, abi: VAULT, functionName: "acquisitionCount" });
const fee = await pub.readContract({ address: d.ponsFactory as Hex, abi: FACTORY, functionName: "launchFee" });
const launched = await mined(await wallet("creator").writeContract({ address: d.launcher as Hex, abi: LAUNCHER, functionName: "launch", value: fee, args: [{ name: "Smoke Stone", symbol: "SMOKE", logo: "",
  description: "a staging smoke test", socials: { twitter: "", telegram: "", discord: "", website: "", farcaster: "" }, creatorTaxBps: 300, expectedEconomics: `0x${"00".repeat(32)}`,
  salt: keccak256(toHex(`smoke-${Date.now()}`)), launchConfigId: 0n, listingId: LISTING }] }));
const ev = launched.logs.filter((l) => l.address.toLowerCase() === d.launcher.toLowerCase()).map((l) => decodeEventLog({ abi: LAUNCHER, data: l.data, topics: l.topics }))[0];
const { coin, curve, router } = ev.args as unknown as { coin: Hex; curve: Hex; router: Hex };
ok("a stranger launched a coin on the forked Pons factory through the Launcher, signing for the staging chain", { chainId: STAGING_CHAIN_ID, coin, router });

await sleep(2_500);
const spends = { alice: 12n * 10n ** 17n, bob: 9n * 10n ** 17n, carol: 6n * 10n ** 17n };
for (const [n, v] of Object.entries(spends)) await mined(await wallet(n).writeContract({ address: curve, abi: CURVE, functionName: "buy", args: [v, 0n, people[n].address], value: v }));
ok("three wallets bought on Pons's real curve");

const h = await mined(await wallet("carol").writeContract({ address: router, abi: ROUTER, functionName: "harvest" }));
const hv = h.logs.filter((l) => l.address.toLowerCase() === router.toLowerCase()).map((l) => decodeEventLog({ abi: ROUTER, data: l.data, topics: l.topics }))[0].args as unknown as { claimed: bigint; toVault: bigint; toTreasury: bigint };
if (hv.toVault + hv.toTreasury !== hv.claimed || hv.toVault !== (hv.claimed * 80n) / 100n) fail(`not 80/20: ${JSON.stringify(hv, (_k, v) => String(v))}`);
ok("harvested by a stranger: exactly 80% to the vault, 20% to the treasury, nothing left in the router", hv);

const acq = await until("the operator to post a cap, declare, wait in public, release against the bond, and settle a FIXTURE receipt", async () => {
  const n = await pub.readContract({ address: vault, abi: VAULT, functionName: "acquisitionCount" });
  for (let i = before; i < n; i++) { const a = await pub.readContract({ address: vault, abi: VAULT, functionName: "acquisition", args: [i] }); if (a.coin.toLowerCase() === coin.toLowerCase()) return { id: i, a }; }
  return null;
});
ok("cap → declaration → public wait (shortened on staging) → release → attested receipt; charged to the coin whose fees paid", { acquisition: acq.id, receipt: "staging-fixture" });

await mined(await wallet("creator").writeContract({ address: d.distributor as Hex, abi: DIST, functionName: "choose", args: [LISTING, acq.id, 3, "0x0000000000000000000000000000000000000000"] }));
ok("the creator chose a balance-weighted draw");
const lotOf = () => pub.readContract({ address: d.distributor as Hex, abi: DIST, functionName: "lot", args: [LISTING, acq.id] });
const opened = await until("the draw to open with the snapshot tool's root", async () => { const l = await lotOf(); return l.stage >= 1 ? l : null; });

// anyone can rebuild the root: the published list, then the chain itself, through the public proxy
const published = (await (await fetch(opened.uri)).json()) as { root: Hex; inputs: { block: string; exclusions: string[]; kind: "weighted"; decimals: number; coin: string } };
const logs = await transferLogs(RPC, coin, BigInt(d.fromBlock), BigInt(published.inputs.block));
const rebuilt = snapshot(balancesOf(logs), published.inputs);
if (rebuilt.root !== opened.root || published.root !== opened.root) fail(`the snapshot does not rebuild: chain ${opened.root}, published ${published.root}, rebuilt ${rebuilt.root}`);
ok("the published snapshot rebuilds byte for byte from the chain's own Transfer logs, read through the public proxy", { root: opened.root, holders: rebuilt.holders });

const pinned = await until("anyone to pin a drand round that does not exist yet", async () => { const l = await lotOf(); return l.stage >= 2 ? l : null; });
const resolved = await until("the REAL drand round to publish and be verified on chain", async () => { const l = await lotOf(); return l.stage >= 3 ? l : null; });
ok("draw pinned to a future drand round, resolved with drand's own signature verified on chain", { round: pinned.round, winningTicket: resolved.winningTicket });
const claimed = await until("the winner to be proven by Merkle proof", async () => { const l = await lotOf(); return l.stage >= 4 ? l : null; });
const winner = Object.entries(people).find(([, a]) => a.address.toLowerCase() === claimed.winner.toLowerCase())?.[0] ?? fail(`the winner ${claimed.winner} is not one of the three buyers`);
await mined(await wallet(winner).writeContract({ address: d.distributor as Hex, abi: DIST, functionName: "setDestination", args: [LISTING, acq.id, "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0"] }));
ok("the winner posted a Bitcoin address (BIP-350's test vector: a fixture nobody holds)", { winner });
const delivered = await until("the delivery to be recorded", async () => { const l = await lotOf(); return l.stage === 5 ? l : null; });
ok("FIXTURE delivery recorded; the lot ended Delivered to the proven winner", { deliveryTxid: delivered.deliveryTxid });

if (API) {
  const get = async <T>(p: string) => (await (await fetch(`${API}${p}`)).json()) as T;
  await until("the indexer to reach the delivery", async () => { const x = await get<{ stage?: string; lot?: { stage?: string } }>(`/v1/draws/${LISTING}-${acq.id}`); return /deliver/i.test(JSON.stringify(x)) ? x : null; }, 4_000);
  const c = await get<Record<string, unknown>>(`/v1/coins/${coin.toLowerCase()}`);
  const s = await get<{ coins: number; acquisitions: { inscriptions: number } }>("/v1/stats");
  if (!JSON.stringify(c).toLowerCase().includes(router.toLowerCase()) || s.coins < 1 || s.acquisitions.inscriptions < 1) fail("the API does not reflect the run");
  const hl = await get<{ deployment: boolean; chainId: number }>("/v1/health");
  if (!hl.deployment || hl.chainId !== STAGING_CHAIN_ID) fail("the API is not following the staging deployment");
  ok("the indexer's API reflects the coin, the acquisition and the delivered draw, over the public proxy", { coins: s.coins, inscriptions: s.acquisitions.inscriptions });
}
void readFileSync;
console.log(`\nstaging smoke OK — ${steps.length} checks`);
process.exit(0);
