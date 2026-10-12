// One alert of each kind, for `--test-alert` (G9: "a test alert of each kind reaches the operator's phone"). Each is produced
// by the real code path from the fixtures (monitor/fixtures), so what arrives is exactly what a real alert looks like. Nothing
// here reads a network or touches the state file.
import { readFileSync } from "node:fs";
import { describe } from "../../scripts/timelock-watch.mjs";
import { KINDS, heartbeat } from "./alerts.mjs";
import { proposalAlert } from "./argdiff.mjs";
import { decodeInit, decodeReply } from "./candid.mjs";
import { judgeAddress } from "./canary.mjs";
import { judgeCanister } from "./custodyhealth.mjs";
import { drawOutcome, guardianAlert, judgeLogs, judgeStackLogs } from "./evm.mjs";
import { stackOf } from "./stack.mjs";
import { recordIndex, EMPTY_RECORDS } from "./records.mjs";
import { certifiedAlerts, cyclesCheck } from "./watcher.mjs";

const F = (name) => JSON.parse(readFileSync(new URL(`../fixtures/${name}`, import.meta.url), "utf8"));
const TL = "aaaaa-bbbbb-ccccc-ddddd-cai-TEST", BUYER = "eeeee-fffff-ggggg-hhhhh-cai-TEST";
const ADDR = "bc1pfssfalzdt9h8uwqcrs73780l723mh2axrul7keaevke7a8pg4a7szrd0ht";   // a real mainnet address, used as a sample only
const PROVIDERS = [{ name: "mempool.space", base: "https://mempool.space/api", file: "esplora/mempool-address-txs.json" },
  { name: "blockstream.info", base: "https://blockstream.info/api", file: "esplora/blockstream-address-txs.json" }];

export function sample(kind, now = Date.now()) {
  const c = F("canister.json");
  const views = () => PROVIDERS.map((p) => ({ name: p.name, base: p.base, txs: F(p.file) }));
  const records = (redemptions, frogs = true) => ({ ...EMPTY_RECORDS, redemptions: decodeReply("redemptions", c.replies[redemptions]),
    frog_buys: frogs ? decodeReply("frog_buys", c.replies.frog_buys) : [] });
  const one = (alerts, k) => { const a = alerts.find((x) => x.kind === k); if (!a) throw new Error(`no ${k} sample produced`); return a; };
  switch (kind) {
    case "proposal": {
      const v = { id: "7", by: "p1-TEST", kind: "upgrade", wasm_hash: ["f".repeat(64)], wasm_bytes: ["1603201"], arg_hex: [c.args.loosened],
        arg_hash: ["e".repeat(64)], controllers: [], roles: [], proposed_ns: String(BigInt(now) * 1_000_000n),
        executable_ns: String((BigInt(now) + 259_200_000n) * 1_000_000n), vetoes: [] };
      return proposalAlert(v, { timelock: TL, builds: [{ commit: "abc1234", buyer: "a".repeat(64), timelock: "b".repeat(64) }],
        baseline: { init: decodeInit(c.args.install).init, source: "the install argument recorded at the ceremony" },
        decoded: decodeInit(c.args.loosened), roles: null, controllers: [TL] });
    }
    case "governance":
      return { kind, key: "test", sev: "INFO", text: describe({ Vetoed: { id: "7", at_ns: String(BigInt(now) * 1_000_000n), by: ["g1-TEST", "g2-TEST"] } }, { timelock: TL }) };
    case "controllers":
      return one(certifiedAlerts({ buyer: BUYER, timelock: TL, builds: [{ commit: "abc1234", buyer: "a".repeat(64), timelock: "b".repeat(64) }],
        now: { [BUYER]: { controllers: [TL], module_hash: "c".repeat(64) }, [TL]: { controllers: [], module_hash: "b".repeat(64) } },
        prev: { [BUYER]: { controllers: [TL], module_hash: "a".repeat(64) }, [TL]: { controllers: [], module_hash: "b".repeat(64) } },
        executed: false, pending: [] }).alerts.filter((a) => a.key.startsWith("unexecuted")), "controllers");
    case "config":
      return { kind, key: "test", sev: "SEV2", text: `custody is FROZEN by the tripwire since ${new Date(now).toISOString()}: {"Tripwire":{"collection":"btc:0x…","flow":{"Item":null},"demanded":"9","limit":"3"}}. G10 SEV2: assess within an hour; resuming needs two guardians.` };
    case "custody-outbound":
      return one(judgeAddress({ label: "custody (TEST SAMPLE)", address: ADDR, views: views(), index: recordIndex(records("redemptions_none_delivered")),
        now, timelock: TL }).alerts, "custody-outbound");
    case "custody-inbound":
      return one(judgeAddress({ label: "custody (TEST SAMPLE)", address: ADDR, views: views(), index: recordIndex(records("redemptions_delivered", false)),
        now, timelock: TL, expect: [{ id: "custody#0", min_sats: 1000, note: "the G10 drill's test deposit" }] }).alerts, "custody-inbound");
    case "provider-disagreement": {
      const v = views();
      v[1].txs = v[1].txs.filter((t) => !t.txid.startsWith("2127bc90"));
      return one(judgeAddress({ label: "custody (TEST SAMPLE)", address: ADDR, views: v, index: recordIndex(records("redemptions_none_delivered")),
        now, timelock: TL }).alerts, "provider-disagreement");
    }
    case "evm-mint": {
      const e = F("evm-logs.json");
      return one(judgeLogs({ logs: [e.logs.deposited_unknown], index: recordIndex(records("redemptions_delivered")),
        cfg: { wrappers: [e.wrapper], floor_module: e.module }, timelock: TL }).alerts, "evm-mint");
    }
    case "cycles":
      return one(cyclesCheck({}, { [`the buyer ${BUYER}`]: { balance: "1900000000000", low_threshold: "5000000000000" } }).alerts, "cycles");
    case "watcher-error":
      return { kind, key: "test", sev: "SEV2", text: "the canary checker has not completed a poll for 34 min (last error: mempool.space answered HTTP 503). Silence is not safety: check the monitor's /status." };
    case "keeper": {
      const S = F("stack-logs.json"), st = stackOf({ stack: S.stack });
      return one(judgeStackLogs({ logs: [S.logs.floor_clamped_1], ctx: { st, now } }).alerts, "keeper");
    }
    case "draw": {
      const S = F("stack-logs.json"), st = stackOf({ stack: S.stack });
      const d = { vault: st.collections[0].raffle, id: "0", coin: S.coin, slug: "frogs", root: `0x${"ab".repeat(32)}`, totalTickets: "1000", snapshotAt: "1789990000" };
      const ra = { root: `0x${"cd".repeat(32)}`, totalTickets: 1000n, block: 71_199_000, holders: 40 };
      return one(drawOutcome("sample", d, { v: "bad", why: `recomputed ${ra.root} with 1000 tickets at block 71199000 (snapshot time 1789990000); the chain has ${d.root} with 1000` }, ra, st).alerts, "draw");
    }
    case "guardian": {
      const S = F("stack-logs.json"), st = stackOf({ stack: S.stack });
      return guardianAlert({ key: "sample", action: "pause", why: "a runaway frogs floor: 2 posts outside the module's bounds, all down, within 6.0 h" }, { done: true, tx: `0x${"7".repeat(64)}` }, null, st);
    }
    case "liveness":
      return { kind, key: "test", sev: "SEV2", text: "keeper is DOWN or refusing: https://keeper.example/healthz has not answered 200 for 12 min (HTTP 503). Railway does not restart a running service whose /healthz fails; check its logs and /status." };
    case "canister":
      return one(judgeCanister({ buyer: BUYER, read: { ticketLogs: [], log: [], ordProviders: [["ord", "https://ordinals.com"], ["ordinalswallet", "https://turbo.ordinalswallet.com"]], holderDeposits: [] }, now }).alerts, "canister");
    case "fueler":
      return { kind, key: "test", sev: "SEV1", persistent: true, text: "ops-unexpected-tx: the ops address 0xabababababababababababababababababababab has sent 6 transaction(s) on chain 4663, but the fueler journaled only 5. Something else signed with the ops key. TREAT IT AS STOLEN: the Safe calls treasury.setOps(<a fresh address>) now, then rotate FUELER_EVM_KEY (docs/v7/cycles-from-fees.md §3.4)." };
    case "heartbeat":
      return heartbeat(now, { watched: [`timelock ${TL}, buyer ${BUYER}`, `custody ${ADDR}`], checkers: { watcher: { lastOk: now - 60_000 }, canary: { lastOk: now - 60_000 }, evm: { lastOk: now - 60_000 } }, log: [] });
    default:
      throw new Error(`unknown alert kind ${kind}; the kinds are: ${KINDS.join(", ")}`);
  }
}

export const samples = (now = Date.now()) => KINDS.map((k) => sample(k, now));
