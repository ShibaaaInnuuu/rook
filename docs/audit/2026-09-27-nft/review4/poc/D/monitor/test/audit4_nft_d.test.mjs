// Review round 4 (2026-09-27 NFT audit), part D: services. No network. Run from a private copy of monitor/ (node --test), with
// test/d4-state.hex written by review4/poc/D/canister/d4_fixture.rs (the canister's own encoder).
//  (1) F3-1 end to end: eth_nft_state() bytes written by the CANISTER, decoded by the monitor's IDL and turned into plain() JSON
//      exactly as ic.mjs does, fed to every check that reads it. "What holds" (passes = the fix holds).
//  (2) D4-L-1: the one-tick false SEV1 (a Sent delivery whose NFT the redeemer already moved on).
//  (3) D4-L-2: the incremental deposit scan never re-reads a block its provider answered short (load-balanced lag / tip reorg).
//  (4) D4-I-1: the exemption never checks the ticket's stage (a refunded ticket's stale Moved delivery).
//  (5) D4-I-2: /status's alerts_last_24h head is not redacted; a cut secret is not recognised.
// A passing weakness test means the weakness is present.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { IDL } from "@icp-sdk/core/candid";
import { encodeFunctionData, encodeFunctionResult, parseAbi } from "viem";
import { EthState, plain } from "../src/candid.mjs";
import { checkEthNft, earmarkOf, ethNftAlerts } from "../src/ethnft.mjs";
import { TOPICS, judgeLogs } from "../src/evm.mjs";
import { EMPTY_RECORDS, recordIndex } from "../src/records.mjs";
import { expandSecrets, redact } from "../src/config.mjs";
import { deliver } from "../src/alerts.mjs";
import { statusBody } from "../src/main.mjs";

const E18 = 10n ** 18n;
const A = (n) => `0x${n.toString(16).padStart(40, "0")}`;
const rep = (b, n = 20) => `0x${b.repeat(n)}`;
const WRAPPER = rep("aa"), MODULE = rep("bb"), DEST = rep("d0"), CUSTODIAN = A(0xc0), POSTER = A(0xf5), MILADY = "0x5af0d9827e0c53e4799bb226655a1de152a425a5";
const BUYER2 = A(0xb0b), THIEF = A(0xbad);
const NOW = 1_800_000_000_000;
const pad = (x) => `0x${BigInt(x).toString(16).padStart(64, "0")}`;
const padA = (a) => `0x${"0".repeat(24)}${a.slice(2)}`;

const WRAP = parseAbi(["function itemCount() view returns (uint256)", "function item(uint256) view returns ((uint256 tokenId, bytes32 sourceTx, bool deposit))",
  "function itemOf(uint256) view returns (uint256)",
  "function ticket(uint256) view returns ((uint8 stage, address owner, uint64 round, uint64 pinnedAt, uint64 pinnedAtBlock, uint64 deliverBy, uint64 deliverByBlock, uint256 itemId, bytes32 deliveryTx, address destination))"]);
const MOD = parseAbi(["function accepted() view returns (uint256)", "function netUnits() view returns (uint256)", "function buffer() view returns (uint256)",
  "function maxItemCost() view returns (uint256)", "function registry() view returns (address)"]);
const ERC = parseAbi(["function ownerOf(uint256) view returns (address)"]);
const sel = (abi, fn, args = []) => encodeFunctionData({ abi, functionName: fn, args }).slice(0, 10);
const err = (message) => new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, error: { code: -32000, message } }));
/** `logs(from, to)`: what eth_getLogs answers for a range; `head()`: what eth_blockNumber answers. */
function world({ items = [], owner = () => CUSTODIAN, head = () => 26_000_000, balance = 10n ** 17n, nonce = 7, tickets = {}, logs = () => [] }) {
  const calls = [];
  const http = async (url, init) => {
    if (url.includes("opensea.io")) return new Response(JSON.stringify({ total: { floor_price: 0.93 } }));
    if (url.includes("coingecko")) return new Response(JSON.stringify({ floor_price: { native_currency: 0.93 } }));
    const b = JSON.parse(String(init.body));
    calls.push(b.method);
    const ok = (result) => new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, result }));
    if (b.method === "eth_blockNumber") return ok(`0x${head().toString(16)}`);
    if (b.method === "eth_getBalance") return ok(`0x${balance.toString(16)}`);
    if (b.method === "eth_getTransactionCount") return ok(`0x${nonce.toString(16)}`);
    if (b.method === "eth_getLogs") { const p = b.params[0]; const a = Number(BigInt(p.fromBlock)), z = Number(BigInt(p.toBlock)); calls.push(`logs:${a}-${z}`); return ok(logs(a, z)); }
    const { data } = b.params[0], s = data.slice(0, 10);
    if (url === "https://home") {
      if (s === sel(WRAP, "itemCount")) return ok(encodeFunctionResult({ abi: WRAP, functionName: "itemCount", result: BigInt(items.length) }));
      if (s === sel(WRAP, "item", [0n])) { const i = Number(BigInt(`0x${data.slice(10)}`)); return ok(encodeFunctionResult({ abi: WRAP, functionName: "item", result: { tokenId: items[i], sourceTx: `0x${"11".repeat(32)}`, deposit: false } })); }
      if (s === sel(WRAP, "itemOf", [0n])) { const t = BigInt(`0x${data.slice(10)}`); return ok(encodeFunctionResult({ abi: WRAP, functionName: "itemOf", result: BigInt(items.indexOf(t) + 1) })); }
      if (s === sel(WRAP, "ticket", [0n])) {
        const id = String(BigInt(`0x${data.slice(10)}`)), t = tickets[id];
        calls.push(`ticket:${id}`);
        if (!t) return err("execution reverted");
        return ok(encodeFunctionResult({ abi: WRAP, functionName: "ticket", result: { stage: t.stage ?? 2, owner: A(0x1), round: 1n, pinnedAt: 1n, pinnedAtBlock: 1n, deliverBy: 1n,
          deliverByBlock: 1n, itemId: t.itemId, deliveryTx: `0x${"00".repeat(32)}`, destination: t.destination ?? DEST } }));
      }
      if (s === sel(MOD, "accepted")) return ok(encodeFunctionResult({ abi: MOD, functionName: "accepted", result: 93n * E18 / 100n }));
      if (s === sel(MOD, "registry")) return ok(encodeFunctionResult({ abi: MOD, functionName: "registry", result: POSTER }));
      for (const fn of ["netUnits", "buffer", "maxItemCost"]) if (s === sel(MOD, fn)) return ok(encodeFunctionResult({ abi: MOD, functionName: fn, result: 0n }));
    }
    if (s === sel(ERC, "ownerOf", [0n])) return ok(encodeFunctionResult({ abi: ERC, functionName: "ownerOf", result: owner(url, BigInt(`0x${data.slice(10)}`)) }));
    throw new Error(`unexpected ${url} ${b.method} ${s}`);
  };
  return { http, calls };
}
const cfgOf = (x = {}) => ({ buyer: "rrkah-fqaaa-aaaaa-aaaaq-cai", eth_nft: { home_rpc: ["https://home"], eth_rpc: ["https://a", "https://b"],
  collections: [{ slug: "milady", collection: MILADY, wrapper: WRAPPER, module: MODULE, custodian: CUSTODIAN, opensea_slug: "milady", coingecko_id: "milady-maker" }], ...x } });
const notHeld = (r) => r.alerts.filter((a) => a.sev === "SEV1" && a.key.startsWith("eth-not-held:"));

// the canister's own bytes, as the live monitor sees them (ic.mjs: the actor's IDL decode, then plain())
const live = () => plain(IDL.decode([EthState], Uint8Array.from(Buffer.from(readFileSync(new URL("./d4-state.hex", import.meta.url), "utf8").trim(), "hex")))[0]);

// ---- (1) F3-1 end to end ----------------------------------------------------------------------------------------------------------
test("F3-1 (what holds): the canister-encoded state through plain(): blobs are hex text and every check compares them", async () => {
  const st = live();
  assert.equal(st.deliveries[0].wrapper, "aa".repeat(20), "plain(): hex text without 0x");
  assert.deepEqual(st.deliveries[0].sent, [true]);
  assert.deepEqual(st.buys[3].sweep, [true]);
  // the deadline page reads the mined phase, the tx and the sent note
  const near = 1_800_000_000 - 3600;
  const a = ethNftAlerts({ buyer: "b", state: st, nowS: near }).alerts.filter((x) => x.key.startsWith("eth-deliver-deadline"));
  const by = Object.fromEntries(a.map((x) => [x.key.split(":")[2], x]));
  assert.equal(by["3"].sev, "SEV2", "Moved + noteSent: SEV2");
  assert.equal(by["6"].sev, "SEV1", "Moved, noteSent not landed: SEV1");
  assert.match(by["3"].text, /token 9180 to 0xd0d0d0/);
  assert.ok(by["3"].text.includes(`0x${"33".repeat(32)}`));
  assert.equal(by["4"].sev, "SEV1", "Sent near the deadline: the generic SEV1");
  assert.ok(!by["5"], "Recorded: nothing");
  // the record index: deliveries, mints, advances (the sweep excluded)
  const idx = recordIndex({ ...EMPTY_RECORDS, eth: st });
  assert.deepEqual(idx.ethDeliveries.get(`${WRAPPER}:3`), { phase: "Moved", tx: `0x${"33".repeat(32)}` });
  assert.equal(idx.ethMints.get(`${MILADY}:9182`)[0].to, MODULE);
  assert.equal(idx.ethMints.get(`${MILADY}:9186`)[0].tx, `0x${"77".repeat(32)}`);
  assert.ok(!idx.advances.some((x) => x.frog === "eth:4"), "a sweep is not an advance");
  assert.ok(idx.advances.some((x) => x.frog === "eth:2" && x.module === MODULE));
  // the 4663 events the canister's records back: none pages
  const cfg = { nft_wrappers: [WRAPPER], nft_collection_of: { [WRAPPER]: MILADY } };
  const L = (i, topics, data) => ({ address: WRAPPER, topics, data, blockNumber: "0x10", logIndex: `0x${i.toString(16)}`, transactionHash: `0x${String(i).repeat(64)}` });
  const logs = [
    L(1, [TOPICS.NftDeliverySent, pad(3), pad(9180)], `0x${"33".repeat(32)}`),
    L(2, [TOPICS.NftWrapped, pad(0), pad(9182), padA(MODULE)], `0x${"ac".repeat(32)}`),
    L(3, [TOPICS.NftDeposited, pad(1), pad(9186), padA(rep("12"))], `0x${"77".repeat(32)}`),
  ];
  assert.deepEqual(judgeLogs({ logs, index: idx, cfg }).alerts, [], "plain() records back each event");
  // controls: a note naming another tx, a mint to another address: both SEV1
  const bad = judgeLogs({ logs: [L(4, [TOPICS.NftDeliverySent, pad(3), pad(9180)], `0x${"34".repeat(32)}`), L(5, [TOPICS.NftWrapped, pad(0), pad(9182), padA(THIEF)], `0x${"ac".repeat(32)}`)], index: idx, cfg }).alerts;
  assert.equal(bad.filter((x) => x.sev === "SEV1").length, 2);
  // the outbound canary's earmark from the plain line (Buy{2} at nonce 6, used)
  const line = st.line[0];
  assert.equal(earmarkOf(st.buys[1], line, 7n), 3n * E18 / 10n - (E18 / 4n + 1_000_000_000n * 300_000n));
  assert.equal(earmarkOf(st.buys[3], line, 7n), E18 / 20n, "the sweep's BackFunded ETH is earmarked (it is at our address)");
  // the custody check: the mined delivery (NFT resold) is exempt, the bought token at a thief pages, the deposit is custody
  const w = world({ items: [9180n, 9185n], tickets: { 3: { itemId: 0n }, 6: { itemId: 1n } },
    owner: (u, t) => (t === 9180n ? BUYER2 : t === 9185n ? DEST : t === 9181n ? THIEF : CUSTODIAN) });
  const r = await checkEthNft({ cfg: cfgOf(), now: NOW, http: w.http, ic: { ethState: async () => st } });
  assert.deepEqual(notHeld(r).map((x) => x.key), [`eth-not-held:${MILADY}:9181`]);
  assert.ok(!r.alerts.some((x) => /eth-collection-unread/.test(x.key)), JSON.stringify(r.alerts.map((x) => x.key)));
  // the canary's balance check reads the plain earmarks: 0.1 ETH covers 0.0497 + 0.05; 0.05 does not, and the key names #2 and #4
  const w2 = world({ items: [], balance: 5n * 10n ** 16n });
  const r2 = await checkEthNft({ cfg: cfgOf(), now: NOW, http: w2.http, ic: { ethState: async () => st } });
  assert.ok(r2.alerts.some((x) => x.key === `eth-arrived-gone:${CUSTODIAN}:2,4`), JSON.stringify(r2.alerts.map((x) => x.key)));
  assert.ok(!r.alerts.some((x) => x.key.startsWith("eth-arrived-gone")));
});

// ---- (2) D4-L-1 -------------------------------------------------------------------------------------------------------------------
test("D4-L-1 (weakness): a delivery that mined but is still Sent in the canister (its next tick up to RUNNER_TICK_EVERY_S = 600 s away), NFT already sold on: SEV1", async () => {
  const st = live();
  st.deliveries = [{ ...st.deliveries[1] }];     // ticket 4, token 9183, Sent (its transfer mined a block ago)
  const w = world({ items: [9183n], tickets: { 4: { itemId: 0n } }, owner: (u, t) => (t === 9183n ? BUYER2 : CUSTODIAN) });
  const r = await checkEthNft({ cfg: cfgOf(), now: NOW, http: w.http, ic: { ethState: async () => st } });
  const p = notHeld(r);
  assert.equal(p.length, 1, "WEAKNESS: an honest redemption resold within a tick pages SEV1");
  assert.match(p[0].text, /pause, then look/);
  assert.ok(!w.calls.includes("eth_getTransactionReceipt"), "the monitor never reads the Sent transfer's receipt, which would show it mined");
});

// ---- (3) D4-L-2 -------------------------------------------------------------------------------------------------------------------
const DEPOSIT_TX = `0x${"de".repeat(32)}`;
const depLog = (block) => ({ address: MILADY, blockNumber: `0x${block.toString(16)}`, transactionHash: DEPOSIT_TX, logIndex: "0x0",
  topics: ["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef", padA(A(0x999)), padA(CUSTODIAN), pad(4242)], data: "0x" });
test("D4-L-2 (weakness): the incremental deposit scan skips blocks its provider answered short once; an unclaimed deposit there never pages", async () => {
  const st = live();
  st.deposits = [];      // the runner never handed it in: exactly what this watch exists for
  const H = 26_000_000, D = H - 1;
  let head = H, backend = H - 2;    // one provider URL, two nodes: eth_blockNumber answered by a node at H, eth_getLogs by one at H - 2
  const logs = (a, b) => (D >= a && D <= Math.min(b, backend) ? [depLog(D)] : []);
  const mk = () => world({ head: () => head, logs });
  const r1 = await checkEthNft({ cfg: cfgOf(), now: NOW, http: mk().http, ic: { ethState: async () => st } });
  assert.equal(r1.next.scanned[MILADY], String(H), "the cursor moves to the head the OTHER node reported");
  // 700 blocks on (past UNCLAIMED_LATE_BLOCKS = 600), every node caught up long ago
  head = H + 700; backend = head;
  const w2 = mk();
  const r2 = await checkEthNft({ cfg: cfgOf(), now: NOW + 8_400_000, http: w2.http, ic: { ethState: async () => st }, prev: r1.next });
  assert.ok(w2.calls.includes(`logs:${H + 1}-${H + 500}`), "the next poll reads from H + 1 on");
  assert.ok(!r2.alerts.some((a) => a.key.startsWith("eth-unclaimed:")), "WEAKNESS: the unclaimed deposit at H - 1 is never paged");
  // control: a whole-window read (what the code before a37f54b did every poll, and what a fresh start does) pages it
  const r3 = await checkEthNft({ cfg: cfgOf(), now: NOW + 8_400_000, http: mk().http, ic: { ethState: async () => st }, prev: { ...r1.next, scanned: {} } });
  assert.ok(r3.alerts.some((a) => a.key === `eth-unclaimed:rrkah-fqaaa-aaaaa-aaaaq-cai:${DEPOSIT_TX}`), "a whole-window read finds it");
});

// ---- (4) D4-I-1 -------------------------------------------------------------------------------------------------------------------
test("D4-I-1 (note): the exemption reads the ticket's item and destination, not its stage: a REFUNDED ticket's stale Moved delivery exempts its item", async () => {
  const st = live();
  st.deliveries = [{ ...st.deliveries[3] }];     // ticket 6, token 9185, Moved, sent none
  // ticket 6 was refunded (stage 4: its item back in the set, still held); the canister's record is still Moved; the NFT is elsewhere
  const w = world({ items: [9185n], tickets: { 6: { itemId: 0n, stage: 4 } }, owner: (u, t) => (t === 9185n ? THIEF : CUSTODIAN) });
  const r = await checkEthNft({ cfg: cfgOf(), now: NOW, http: w.http, ic: { ethState: async () => st } });
  assert.deepEqual(notHeld(r), [], "no eth-not-held although the wrapper counts item 0 as held and the NFT is not ours");
});

// ---- (5) D4-I-2 -------------------------------------------------------------------------------------------------------------------
test("D4-I-2 (note): an alert's first line is kept unredacted in state.log and served on /status; a cut secret is not recognised", async () => {
  const cfg = { eth_nft: { eth_rpc: ["https://api-ethereum-mainnet.n.dwellir.com/${D4_KEY}", "https://xrpc.cl/eth"] } };
  expandSecrets(cfg, { D4_KEY: "0123456789abcdef-SECRET" });
  const url = cfg.eth_nft.eth_rpc[0];
  assert.equal(redact(`x ${url}`), "x https://api-ethereum-mainnet.n.dwellir.com/<redacted>", "the Telegram text is redacted");
  const state = { sent: {} };
  const channel = { post: async () => true, send: async () => true };
  await deliver([{ kind: "watcher-error", sev: "SEV3", key: "k", text: `a provider error echoing ${url}` }], state, channel, NOW);
  const body = JSON.stringify(statusBody({ timelock: "t", network: "ic" }, state, NOW));
  assert.ok(body.includes("SECRET"), "WEAKNESS (defence in depth): the status body carries the unredacted head");
  assert.ok(redact(url.slice(0, 60)).includes("0123456789"), "a key cut before redaction is not recognised");
});
