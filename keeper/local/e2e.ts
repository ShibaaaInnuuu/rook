import { spawn, spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { decodeEventLog, decodeFunctionResult, encodeFunctionData, keccak256, encodeAbiParameters, toHex, type Abi, type Hex } from "viem";
import { localSender, type LocalSender } from "./guard.js";
import { BEACON, CURVE, DISTRIBUTOR, FACTORY, LAUNCHER, REGISTRY, ROUTER, VAULT } from "./abis.js";
import { plan, type Action } from "../src/plan.js";
import { readListings } from "../src/chain.js";
import { FixtureQuoter, FixtureSource } from "../src/sources.js";
import { balancesOf, serialise, snapshot } from "../../tools/snapshot/src/core.js";
import { transferLogs } from "../../tools/snapshot/src/rpc.js";

/**
 * docs/phase2.md §3 — the whole cycle against a LOCAL fork of Robinhood Chain, ending with a winner's claim and a recorded delivery.
 *   KEEPER_MODE=local npm run e2e:local            (about a minute)
 *   KEEPER_MODE=local npm run e2e:local -- --real-waits   (an hour and a quarter: the two public waits are really waited)
 *
 * What is real: Pons's factory and curve (forked), every etch contract (deployed by script/deploy.sh), the keeper's own policy
 * (src/plan.ts), the snapshot tool's root and proof from the fork's real Transfer logs, and the drand beacon — a round that did not
 * exist when it was pinned, fetched from drand's public HTTP API and verified on chain.
 * What is a FIXTURE, and is labelled so in the journal: the floor price, the ETH→BTC quote, the Bitcoin receipt and the delivery
 * txid. No Bitcoin transaction exists; nothing was bought.
 * What the harness fakes: the two public waits (one hour before a release, fifteen minutes before a draw). By default it does NOT
 * advance the clock — an advanced clock would pin a drand round 75 real minutes away — it rewrites the two stored deadlines with
 * the local node's anvil_setStorageAt. A real chain has neither trick; --real-waits uses neither.
 */
const ROOT = new URL("../../", import.meta.url).pathname;
const RPC = process.env.LOCAL_RPC ?? "http://127.0.0.1:8545";
const REAL_WAITS = process.argv.includes("--real-waits");
const DRAND = "https://api.drand.sh/04f1e9062b8a81f848fded9c12306733282b2727ecced50032187751166ec8c3";
const DEMO_INSCRIPTION = "0000000000000000000000000000000000000000000000000000000000000000i0";
// BIP-350's own mainnet P2TR test vector: a valid bech32m address nobody is known to hold. A fixture destination.
const FIXTURE_TAPROOT = "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0";

const steps: Record<string, unknown>[] = [];
const note = (step: string, o: Record<string, unknown> = {}) => { steps.push({ step, ...o }); console.log(`· ${step}${Object.keys(o).length ? " " + JSON.stringify(o, (_k, v) => (typeof v === "bigint" ? String(v) : v)) : ""}`); };
const sleep = (ms: number) => new Promise((ok) => setTimeout(ok, ms));
const eth = (wei: bigint) => `${(Number(wei / 10n ** 12n) / 1e6).toFixed(6)} ETH`;

async function reachable(): Promise<boolean> {
  try { const r = await fetch(RPC, { method: "POST", headers: { "content-type": "application/json" }, body: '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}' }); return r.ok; } catch { return false; }
}

let node: ReturnType<typeof spawn> | null = null;
if (!(await reachable())) {
  const port = new URL(RPC).port || "8545";
  note("starting a local fork of Robinhood Chain (public RPC), chain id 31337", { port });
  node = spawn("anvil", ["--fork-url", "https://rpc.mainnet.chain.robinhood.com", "--chain-id", "31337", "--port", port, "--silent"],
    { env: { ...process.env, PATH: `${process.env.HOME}/.foundry/bin:${process.env.PATH}` }, stdio: "ignore" });
  for (let i = 0; i < 100 && !(await reachable()); i++) await sleep(250);
}
const stop = () => { if (node && !process.argv.includes("--keep")) node.kill(); };
process.on("exit", stop);

const s: LocalSender = await localSender(RPC);        // refuses unless KEEPER_MODE=local and the node says 31337
const [deployer, owner, treasury, keeper, purchaser, attestor, creator, ...buyers] = await s.accounts();
const holders = buyers.slice(0, 3);

// FINDING (docs/phase2.md): on the real chain anvil's well-known dev addresses carry EIP-7702 delegations to a sweeper contract —
// anything sent to them is taken at once. On the fork that code comes along, so a release to the dev "purchaser" vanished. The
// harness clears the delegation on the local node. The lesson is for production: a role address must be one nobody else has a key to.
for (const a of [deployer, owner, treasury, keeper, purchaser, attestor, creator, ...holders]) await s.request("anvil_setCode", [a, "0x"]);

const read = async <T>(to: string, abi: Abi, functionName: string, args: unknown[] = []): Promise<T> => {
  const data = encodeFunctionData({ abi, functionName, args });
  return decodeFunctionResult({ abi, functionName, data: await s.request<Hex>("eth_call", [{ to, data }, "latest"]) }) as T;
};
const write = (from: string, to: string, abi: Abi, functionName: string, args: unknown[] = [], value = 0n) =>
  s.send({ from, to, data: encodeFunctionData({ abi, functionName, args }), value });
const blockTime = async () => Number(BigInt((await s.request<{ timestamp: Hex }>("eth_getBlockByNumber", ["latest", false])).timestamp));

/** Shortens one recorded wait by rewriting a uint64 inside a storage slot to the latest block's time. Local node only. */
async function shortenWait(contract: string, slot: bigint, bitOffset: bigint, what: string, realSeconds: number, backBy = 0): Promise<void> {
  if (REAL_WAITS) { note(`waiting ${realSeconds} s for real: ${what}`); await sleep(realSeconds * 1000 + 2_000); await s.request("evm_mine"); return; }
  const key = toHex(slot, { size: 32 });
  const word = BigInt(await s.request<Hex>("eth_getStorageAt", [contract, key, "latest"]));
  const mask = ((1n << 64n) - 1n) << bitOffset;
  // `backBy`: for a wait counted FROM a stored instant (not TO a stored deadline), the instant is moved that far into the past
  const next = (word & ~mask) | (BigInt((await blockTime()) - backBy) << bitOffset);
  await s.request("anvil_setStorageAt", [contract, key, toHex(next, { size: 32 })]);
  await s.request("evm_mine");
  note(`HARNESS: shortened a public wait (${what}) by rewriting its stored deadline; a real chain waits ${realSeconds} s`);
}

// ---- 1. deploy, through the guarded wrapper ---------------------------------------------------------------------------------------
const deploy = spawnSync("bash", [`${ROOT}script/deploy.sh`, RPC, deployer], { encoding: "utf8", env: { ...process.env,
  ETCH_DEPLOY_CONFIRM: `deploy-etch:31337:${deployer.toLowerCase()}`, ETCH_OWNER: owner, ETCH_TREASURY: treasury, ETCH_KEEPER: keeper,
  ETCH_PURCHASER: purchaser, ETCH_ATTESTOR: attestor } });
if (deploy.status !== 0) throw new Error(`deploy failed: ${(deploy.stderr || deploy.stdout).slice(-600)}`);
const d = JSON.parse(readFileSync(`${ROOT}deployments/31337.json`, "utf8")) as Record<string, string> & { fromBlock: number };
note("deployed by script/deploy.sh; the deployer owns nothing", { registry: d.registry, launcher: d.launcher, distributor: d.distributor, beacon: d.beacon, owner: d.owner });
const LISTING = 0n;
const vault = await read<string>(d.registry, REGISTRY, "vaultOf", [LISTING]);

// ---- 2. a coin, launched through the Launcher on Pons's real factory ---------------------------------------------------------------
const fee = await read<bigint>(d.ponsFactory, FACTORY, "launchFee");
const salt = keccak256(toHex(`etch-e2e-${Date.now()}`));
const launched = await write(creator, d.launcher, LAUNCHER, "launch", [{ name: "Local Stone", symbol: "STONE", logo: "", description: "a local end-to-end run",
  socials: { twitter: "", telegram: "", discord: "", website: "", farcaster: "" }, creatorTaxBps: 200, expectedEconomics: `0x${"00".repeat(32)}`, salt,
  launchConfigId: 0n, listingId: LISTING }], fee);
const ev = launched.logs.filter((l) => l.address.toLowerCase() === d.launcher.toLowerCase()).map((l) => decodeEventLog({ abi: LAUNCHER, data: l.data as Hex, topics: l.topics as [Hex, ...Hex[]] }))[0];
const { coin, curve, router } = ev.args as unknown as { coin: string; curve: string; router: string };
const onPons = await read<readonly unknown[]>(d.ponsFactory, FACTORY, "getLaunchedToken", [coin]);
if (String(onPons[3]).toLowerCase() !== router.toLowerCase()) throw new Error("Pons does not record our router as the creator-fee recipient");
note("launched on Pons V2 (forked); Pons records the router as the creator-fee recipient", { coin, curve, router, creatorTaxBps: 200 });

// ---- 3. trading on Pons's real curve ------------------------------------------------------------------------------------------------
await sleep(2_000); await s.request("evm_mine");          // past the launch-second snipe tax
const spends = [12n * 10n ** 17n, 9n * 10n ** 17n, 6n * 10n ** 17n];
for (let i = 0; i < holders.length; i++) await write(holders[i], curve, CURVE, "buy", [spends[i], 0n, holders[i]], spends[i]);
note("three wallets bought on the curve", { totalEth: eth(spends.reduce((a, b) => a + b, 0n)) });

// ---- 4. anyone harvests: 80% to the vault, attributed to the coin; 20% to the treasury ---------------------------------------------
await write(holders[2], router, ROUTER, "harvest");
const spendable = await read<bigint>(vault, VAULT, "attributed");
note("harvested by a stranger", { vaultSpendable: eth(spendable) });

// ---- 5. the keeper follows its published policy (src/plan.ts) ----------------------------------------------------------------------
const pricesPath = `${ROOT}keeper/journal/e2e-prices.json`;
mkdirSync(`${ROOT}keeper/journal`, { recursive: true });
writeFileSync(pricesPath, JSON.stringify({ inscriptions: { "demo-stones": "150000" }, runes: { "840000:1": "12" } }));
const price = await new FixtureSource(pricesPath).inscriptionFloor("demo-stones");
const quote = await new FixtureQuoter(3_200_000n, 1_500n, 60n).quote(spendable);
const listing = (await readListings(RPC, d.registry)).find((l) => l.id === Number(LISTING))!;
const actions: Action[] = plan(listing, price, quote, Math.floor(Date.now() / 1000), null);
note("the policy's plan (price and quote are FIXTURES)", { actions: actions.map((a) => a.type) });
const cap = actions.find((a) => a.type === "postCap") as Extract<Action, { type: "postCap" }>;
const decl = actions.find((a) => a.type === "declare") as Extract<Action, { type: "declare" }>;
if (!cap || !decl) throw new Error(`the policy did not plan a cap and a declaration: ${JSON.stringify(actions)}`);
await write(keeper, vault, VAULT, "postCap", [BigInt(cap.capWei)]);
await write(keeper, vault, VAULT, "declarePurchase", [BigInt(decl.maxEth), BigInt(decl.quantity), decl.capRef]);
const purchaseId = (await read<bigint>(vault, VAULT, "purchaseCount")) - 1n;
note("cap posted and purchase declared, in public", { capWei: cap.capWei, maxEth: eth(BigInt(decl.maxEth)), quantity: decl.quantity, purchaseId });

// ---- 6. the bond, the public hour, the release -------------------------------------------------------------------------------------
await write(purchaser, vault, VAULT, "postBond", [], BigInt(decl.maxEth));
// AssetVault._purchases is slot 17 (`forge inspect AssetVault storage`; it was 12 before docs/phase3.md H1 and M1); a Purchase is 7 slots; releasableAt is the uint64 after the 1-byte status in its first slot
const purchasesBase = BigInt(keccak256(toHex(17n, { size: 32 })));
await shortenWait(vault, purchasesBase + purchaseId * 7n, 8n, "the hour between a declaration and its release", 3600);
const before = BigInt(await s.request<Hex>("eth_getBalance", [purchaser, "latest"]));
await write(holders[0], vault, VAULT, "release", [purchaseId]);       // anyone may release
const got = BigInt(await s.request<Hex>("eth_getBalance", [purchaser, "latest"])) - before;
if (got !== BigInt(decl.maxEth)) throw new Error("the purchaser did not receive exactly the declared amount");
note("released by a stranger to the purchaser named at declaration; the bond covers it", { released: eth(got) });

// ---- 7. the attestor (a different key) posts a FIXTURE receipt ----------------------------------------------------------------------
const fixtureTxid = keccak256(toHex("FIXTURE: no Bitcoin transaction exists; nothing was bought"));
await write(attestor, vault, VAULT, "settleInscription", [purchaseId, DEMO_INSCRIPTION, fixtureTxid, 150_000n, []]);
const acquisitionId = (await read<bigint>(vault, VAULT, "acquisitionCount")) - 1n;
const acq = await read<{ coin: string; assetRef: string }>(vault, VAULT, "acquisition", [acquisitionId]);
if (acq.coin.toLowerCase() !== coin.toLowerCase()) throw new Error("the acquisition was not charged to the coin whose fees paid");
note("FIXTURE receipt settled by the attestor; charged to the paying coin", { acquisitionId, inscription: acq.assetRef, fixtureTxid });

// ---- 8. the creator chooses a balance-weighted draw; the snapshot tool builds the ticket list --------------------------------------
await write(creator, d.distributor, DISTRIBUTOR, "choose", [LISTING, acquisitionId, 3, "0x0000000000000000000000000000000000000000"]);
const head = BigInt(await s.request<Hex>("eth_blockNumber"));
const logs = await transferLogs(RPC, coin, BigInt(d.fromBlock), head);
const listed = (JSON.parse(readFileSync(`${ROOT}tools/snapshot/exclusions.json`, "utf8")) as { address: string }[]).map((x) => x.address);
const snap = snapshot(balancesOf(logs), { coin, block: String(head), kind: "weighted", decimals: 18,
  exclusions: ["0x0000000000000000000000000000000000000000", "0x000000000000000000000000000000000000dead", coin, curve, router, vault, d.launcher, ...listed] });
const snapName = `snapshot-${coin.toLowerCase()}-${head}-weighted.json`;
writeFileSync(`${ROOT}keeper/journal/${snapName}`, serialise(snap));
note("snapshot built by tools/snapshot from the fork's Transfer logs", { holders: snap.holders, totalTickets: snap.totalTickets, root: snap.root, transfers: logs.length });
await write(keeper, d.distributor, DISTRIBUTOR, "openDraw", [LISTING, acquisitionId, snap.root, BigInt(snap.totalTickets!), `local:${snapName}`]);

// ---- 9. fifteen public minutes, then anyone pins a drand round that does not exist yet ---------------------------------------------
// Distributor._lots is slot 0: keccak(acquisitionId . keccak(listingId . 0)); challengeEnds is the uint64 after three 1-byte fields
const inner = keccak256(encodeAbiParameters([{ type: "uint256" }, { type: "uint256" }], [LISTING, 0n]));
const lotSlot = BigInt(keccak256(encodeAbiParameters([{ type: "uint256" }, { type: "bytes32" }], [acquisitionId, inner])));
await shortenWait(d.distributor, lotSlot, 24n, "the fifteen-minute challenge window of a snapshot", 900);
await write(holders[1], d.distributor, DISTRIBUTOR, "requestDraw", [LISTING, acquisitionId]);
type Lot = { stage: number; round: bigint; winner: string; winningTicket: bigint; destination: string; deliveryTxid: Hex };
let lot = await read<Lot>(d.distributor, DISTRIBUTOR, "lot", [LISTING, acquisitionId]);
const due = Number(await read<bigint>(d.beacon, BEACON, "timeOf", [lot.round]));
const lead = due - Math.floor(Date.now() / 1000);
note("draw pinned by a stranger to a drand round", { round: lot.round, dueInSeconds: lead, existedWhenPinned: lead <= 0 });
if (!REAL_WAITS && lead <= 0) throw new Error("the pinned round already existed: the local clock is behind real time, which the test does not allow");

// ---- 10. the round publishes; its signature comes from drand's public HTTP API and is verified ON CHAIN ----------------------------
type Beacon = { round: number; signature: string; randomness: string };
let beacon: Beacon | null = null;
for (let i = 0; i < 120 && !beacon; i++) {
  await sleep(i === 0 ? Math.max(0, lead) * 1000 + 1_000 : 1_500);
  const r = await fetch(`${DRAND}/public/${lot.round}`, { headers: { "user-agent": "etch-keeper-local/1" } }).catch(() => null);
  if (r?.ok) { const j = (await r.json()) as Beacon | null; if (j && Number(j.round) === Number(lot.round)) beacon = j; }
}
if (!beacon) throw new Error(`drand did not publish round ${lot.round} in time`);
await write(holders[2], d.distributor, DISTRIBUTOR, "resolve", [LISTING, acquisitionId, `0x${beacon.signature}`]);
const proven = await read<Hex>(d.beacon, BEACON, "randomnessOf", [lot.round]);
if (proven.toLowerCase() !== `0x${beacon.randomness}`.toLowerCase()) throw new Error("the chain's randomness is not drand's own");
lot = await read<Lot>(d.distributor, DISTRIBUTOR, "lot", [LISTING, acquisitionId]);
note("REAL drand beacon fetched from api.drand.sh, verified on chain by DrandEvmnet, delivered by a stranger", { round: beacon.round, randomness: `0x${beacon.randomness}`, winningTicket: lot.winningTicket });

// ---- 11. anyone proves the winner from the published list; the winner names a Bitcoin address; the keeper records a delivery -------
const entry = snap.entries.find((e) => BigInt(e.first!) <= lot.winningTicket && lot.winningTicket < BigInt(e.end!))!;
await write(holders[0], d.distributor, DISTRIBUTOR, "claimWin", [LISTING, acquisitionId, entry.holder, BigInt(entry.first!), BigInt(entry.end!), entry.proof]);
const accounts = (await s.accounts()).map((a) => a.toLowerCase());
if (!accounts.includes(entry.holder.toLowerCase())) throw new Error(`the winner ${entry.holder} is not one of the local accounts that bought`);
await write(entry.holder, d.distributor, DISTRIBUTOR, "setDestination", [LISTING, acquisitionId, FIXTURE_TAPROOT]);
// docs/phase3.md L3: for an hour the winner may correct the address, and the keeper may not record a delivery; destinationAt is the
// first uint64 of the lot's SECOND slot (the first is full: three 1-byte fields and three uint64s)
await shortenWait(d.distributor, lotSlot + 1n, 0n, "the hour in which a winner may still correct their Bitcoin address", 3600, 3601);
const fixtureDelivery = keccak256(toHex("FIXTURE: no inscription was sent; nothing exists to send"));
await write(keeper, d.distributor, DISTRIBUTOR, "recordDelivery", [LISTING, acquisitionId, fixtureDelivery]);
lot = await read<Lot>(d.distributor, DISTRIBUTOR, "lot", [LISTING, acquisitionId]);
if (lot.stage !== 5 || lot.winner.toLowerCase() !== entry.holder.toLowerCase()) throw new Error("the lot did not end Delivered to the proven winner");
note("winner proven by Merkle proof from the snapshot tool, destination posted by the winner, FIXTURE delivery recorded", { winner: lot.winner, tickets: `${entry.first}–${entry.end} of ${snap.totalTickets}`, destination: lot.destination });

const out = { at: new Date().toISOString(), rpc: RPC, realWaits: REAL_WAITS, result: "ok", coin, curve, router, vault, listing: Number(LISTING),
  acquisitionId: Number(acquisitionId), drandRound: beacon.round, winner: lot.winner, fixtures: ["floor price", "ETH→BTC quote", "Bitcoin receipt", "delivery txid", "destination address"], steps };
writeFileSync(`${ROOT}keeper/journal/e2e-latest.json`, JSON.stringify(out, (_k, v) => (typeof v === "bigint" ? String(v) : v), 2) + "\n");
// the web app's DEMO mode reads these two files and nothing else
mkdirSync(`${ROOT}web/public/local`, { recursive: true });
if (existsSync(`${ROOT}web`)) {
  copyFileSync(`${ROOT}deployments/31337.json`, `${ROOT}web/public/local/deployment.json`);
  copyFileSync(`${ROOT}keeper/journal/${snapName}`, `${ROOT}web/public/local/${snapName}`);
  writeFileSync(`${ROOT}web/public/local/demo.json`, JSON.stringify({ coin, listing: Number(LISTING), acquisitionId: Number(acquisitionId), snapshot: snapName }, null, 2) + "\n");
}
console.log(`\ne2e:local OK — ${steps.length} steps; journal keeper/journal/e2e-latest.json`);
stop();
process.exit(0);
