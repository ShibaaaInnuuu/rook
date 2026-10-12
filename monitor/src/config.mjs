// The monitor's configuration: public identifiers only (canister ids, addresses, RPC URLs, published hashes). The secrets are never
// here: the Telegram bot token is read from ALERT_TELEGRAM_BOT_TOKEN, and a keyed RPC URL names its key as `${ENV_NAME}` (the
// operator's Dwellir Ethereum key, 2026-09-27: `https://api-ethereum-mainnet.n.dwellir.com/${DWELLIR_API_KEY}`), expanded from the
// environment at load and scrubbed (`redact`) from every alert, status body and log line.
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** tools/snapshot/exclusions.json: the list the keeper's snapshots use, shipped in the monitor's image at the same relative path. */
export const EXCLUSIONS_FILE = fileURLToPath(new URL("../../tools/snapshot/exclusions.json", import.meta.url));

const CANISTER = /^[a-z0-9]{5}(-[a-z0-9]{5}){3,}-[a-z0-9]{3}$/;
const EVM = /^0x[0-9a-fA-F]{40}$/;
const BTC = /^(bc1|tb1)[02-9ac-hj-np-z]{11,87}$/;

/** Read the config: `--config <file>`, else MONITOR_CONFIG (a path), else MONITOR_CONFIG_JSON (the JSON itself, for a host
 *  whose only file is the image). `builds_file` is read relative to the config file. Throws with every problem at once. */
export function loadConfig({ path = process.env.MONITOR_CONFIG, json = process.env.MONITOR_CONFIG_JSON } = {}) {
  let cfg, base = process.cwd();
  if (path) { cfg = JSON.parse(readFileSync(path, "utf8")); base = dirname(resolve(path)); }
  else if (json) cfg = JSON.parse(json);
  else throw new Error("no configuration: pass --config <file>, or set MONITOR_CONFIG or MONITOR_CONFIG_JSON");
  if (cfg.builds_file && !cfg.builds) {
    const f = resolve(base, cfg.builds_file);
    if (!existsSync(f)) throw new Error(`builds_file ${f} does not exist`);
    cfg.builds = JSON.parse(readFileSync(f, "utf8")).builds ?? [];
  }
  // review round 5 (option (c)): the launch file (its `stages`), so a proposal that is exactly a staged step pages as expected
  if (cfg.launch_file && !cfg.launch) {
    const f = resolve(base, cfg.launch_file);
    if (!existsSync(f)) throw new Error(`launch_file ${f} does not exist`);
    cfg.launch = JSON.parse(readFileSync(f, "utf8"));
  }
  if (cfg.evm?.stack_file && !cfg.evm.stack) {
    const f = resolve(base, cfg.evm.stack_file);
    if (!existsSync(f)) throw new Error(`evm.stack_file ${f} does not exist`);
    cfg.evm.stack = JSON.parse(readFileSync(f, "utf8"));
  }
  if (cfg.evm) listedExclusions(cfg, base);
  expandSecrets(cfg);
  return validate(cfg);
}

/** Every secret value an RPC URL was expanded with: `redact` replaces each with `<redacted>`. */
const SECRETS = new Set();

/** A text with every registered secret replaced (alerts, the status body, log lines). */
export function redact(text) {
  let s = String(text);
  for (const v of SECRETS) if (v) s = s.split(v).join("<redacted>");
  return s;
}

/**
 * A provider URL as it may be printed (docs/audit/2026-09-27-nft/bayc10-decisions.md §5): `scheme://host[:port]`, plus "/…" when a
 * path, query, fragment or userinfo was cut. A provider's key lives in exactly those (Dwellir: the path), and `redact` knows only the
 * values expanded from `${ENV_NAME}`, not a key written into a URL (the canister's upgrade argument, a config typed by hand). A text
 * that is not a URL is never printed.
 */
export function redactUrl(u) {
  let x;
  try { x = new URL(String(u)); } catch { return "<not a URL>"; }
  if (!x.host) return "<not a URL>";
  const cut = x.username || x.password || (x.pathname && x.pathname !== "/") || x.search || x.hash;
  return `${x.protocol}//${x.host}${cut ? "/…" : ""}`;
}

/**
 * `${ENV_NAME}` in an RPC URL (eth_nft.eth_rpc and home_rpc, evm.rpcs, draws.providers[].url) is the environment's value, which is
 * never written to a file: a keyed provider's key stays out of the config and out of git. An unset variable refuses the config.
 */
export function expandSecrets(cfg, env = process.env) {
  const one = (u) => typeof u !== "string" ? u : u.replace(/\$\{([A-Z0-9_]+)\}/g, (_, name) => {
    const v = env[name];
    if (!v) throw new Error(`the configuration is refused:\n- ${name} is not set: an RPC URL names it (\${${name}}), and a key is read from the environment only`);
    SECRETS.add(v);
    return v;
  });
  if (cfg.eth_nft) {
    if (Array.isArray(cfg.eth_nft.eth_rpc)) cfg.eth_nft.eth_rpc = cfg.eth_nft.eth_rpc.map(one);
    if (Array.isArray(cfg.eth_nft.home_rpc)) cfg.eth_nft.home_rpc = cfg.eth_nft.home_rpc.map(one);
  }
  if (Array.isArray(cfg.evm?.rpcs)) cfg.evm.rpcs = cfg.evm.rpcs.map(one);
  for (const p of cfg.draws?.providers ?? []) if (p && typeof p.url === "string") p.url = one(p.url);
  return cfg;
}

/**
 * R2-L2: the draw exclusions come from the published list (tools/snapshot/exclusions.json, or `draws.exclusions_file`), never a
 * hand copy that can drift into a self-inflicted cancel. A `draws.exclusions` in the config must equal the file (as a set, any
 * case): otherwise the config is refused. Sets cfg.draws.exclusions (the file's addresses) and cfg.draws.exclusions_sha256.
 */
export function listedExclusions(cfg, base = process.cwd()) {
  cfg.draws ??= {};
  const f = cfg.draws.exclusions_file ? resolve(base, cfg.draws.exclusions_file) : EXCLUSIONS_FILE;
  if (!existsSync(f)) throw new Error(`the draw exclusions file ${f} does not exist (tools/snapshot/exclusions.json, shipped in the image)`);
  const raw = readFileSync(f);
  const list = JSON.parse(raw.toString("utf8")).map((x) => (typeof x === "string" ? x : x.address));
  const norm = (xs) => [...new Set(xs.map((x) => String(typeof x === "string" ? x : x?.address).toLowerCase()))].sort();
  if (cfg.draws.exclusions && JSON.stringify(norm(cfg.draws.exclusions)) !== JSON.stringify(norm(list)))
    throw new Error(`the configuration is refused:\n- draws.exclusions differs from ${f}; remove it from the config (the file is read), or fix the file: a drifted list makes every honest draw "bad"`);
  cfg.draws.exclusions = list;
  cfg.draws.exclusions_file = f;
  cfg.draws.exclusions_sha256 = createHash("sha256").update(raw).digest("hex");
  return list;
}

export function validate(cfg) {
  const bad = [];
  cfg.network ??= "ic";
  cfg.every_secs ??= 60;
  cfg.heartbeat_hour_utc ??= 12;
  cfg.state ??= "/data/monitor-state.json";
  if (!CANISTER.test(cfg.timelock ?? "")) bad.push("timelock: a canister id is required");
  if (cfg.buyer && !CANISTER.test(cfg.buyer)) bad.push("buyer: not a canister id");
  if (!["ic", "local"].includes(cfg.network)) bad.push("network: ic or local");
  if (!(cfg.every_secs >= 30)) bad.push("every_secs: at least 30");
  if (!Array.isArray(cfg.builds) || !cfg.builds.length) bad.push("builds (or builds_file): the published reproducible-build hashes are required; without them every module is UNKNOWN");
  for (const b of cfg.builds ?? []) if (!b.commit || !/^[0-9a-f]{64}$/.test(b.buyer ?? "") || !/^[0-9a-f]{64}$/.test(b.timelock ?? "")) bad.push(`builds: ${JSON.stringify(b)} needs commit, buyer and timelock (64 hex each)`);
  if (cfg.install_arg_hex && !/^4449444c([0-9a-f]{2})*$/i.test(cfg.install_arg_hex)) bad.push("install_arg_hex: Candid bytes as hex (starting 4449444c)");
  cfg.min_fee_pool_sats ??= 20_000;
  if (!(Number.isSafeInteger(cfg.min_fee_pool_sats) && cfg.min_fee_pool_sats >= 1)) bad.push("min_fee_pool_sats: the custody fee pool's minimum largest entry, a whole number of sats (default 20000)");
  if ((cfg.bitcoin || cfg.evm) && !cfg.buyer) bad.push("buyer: required when a canary is on (its records are what spends are matched against)");
  const btc = cfg.bitcoin;
  if (btc) {
    if (!Array.isArray(btc.providers) || btc.providers.length !== 2) bad.push("bitcoin.providers: exactly two esplora bases (they must agree)");
    for (const [label, a] of Object.entries(btc.addresses ?? {})) if (!BTC.test(a)) bad.push(`bitcoin.addresses.${label}: not a bech32 address`);
    if (!Object.keys(btc.addresses ?? {}).length) bad.push("bitcoin.addresses: custody, float and buyer");
    for (const e of btc.expect_inbound ?? []) if (!(btc.addresses ?? {})[e.address]) bad.push(`bitcoin.expect_inbound: ${e.address} is not a watched address label`);
  }
  const evm = cfg.evm;
  if (evm) {
    if (!Array.isArray(evm.rpcs) || evm.rpcs.length !== 2) bad.push("evm.rpcs: exactly two 4663 RPCs (they must agree)");
    for (const w of evm.wrappers ?? []) if (!EVM.test(w)) bad.push(`evm.wrappers: ${w} is not an address`);
    if (evm.floor_module && !EVM.test(evm.floor_module)) bad.push("evm.floor_module: not an address");
    if (evm.from_block !== undefined && !Number.isSafeInteger(evm.from_block)) bad.push("evm.from_block: the L2 block number to start from (deployments/4663.json fromBlock)");
    if (evm.registry && !EVM.test(evm.registry)) bad.push("evm.registry: not an address");
    if (evm.chain_id !== undefined && !Number.isSafeInteger(evm.chain_id)) bad.push("evm.chain_id: a number (4663)");
    if (evm.stack) {
      if (!EVM.test(evm.stack.registry ?? "")) bad.push("evm.stack: not a stack file (deployments/stack-<chainid>.json): no registry");
      if (evm.from_block !== undefined && Number.isSafeInteger(evm.stack.fromBlock) && evm.from_block > evm.stack.fromBlock)
        bad.push(`evm.from_block ${evm.from_block} is later than the stack file's fromBlock ${evm.stack.fromBlock}: coins launched before it would be missed and honest draws recompute "bad" (R2-L1); remove it`);
      if (evm.chain_id !== undefined && evm.stack.chainId !== undefined && Number(evm.stack.chainId) !== evm.chain_id) bad.push(`evm.stack is for chain ${evm.stack.chainId}, not evm.chain_id ${evm.chain_id}`);
    }
    const g = evm.guardian;
    if (g) {
      if (!EVM.test(g.address ?? "")) bad.push("evm.guardian.address: the guardian's address (registry.guardian())");
      if (!evm.stack && !evm.registry) bad.push("evm.guardian needs the registry: give evm.stack (or evm.stack_file, or evm.registry)");
      for (const k of ["auto_pause", "auto_cancel", "pause_on_floor_walk"]) if (g[k] !== undefined && typeof g[k] !== "boolean") bad.push(`evm.guardian.${k}: true or false`);
      if (g.pause_on_no_good_post_h !== undefined && !(typeof g.pause_on_no_good_post_h === "number" && g.pause_on_no_good_post_h >= 2 && g.pause_on_no_good_post_h <= 160))
        bad.push("evm.guardian.pause_on_no_good_post_h: hours without a good floor post before the guardian pauses the keeper, 2 to 160 (omit: never)");
    }
    for (const x of cfg.draws?.exclusions ?? []) if (!EVM.test(typeof x === "string" ? x : x?.address ?? "")) bad.push(`draws.exclusions: ${JSON.stringify(x)} is not an address (or { address, why })`);
    const d = cfg.draws ?? {};
    for (const p of d.providers ?? []) {
      const u = typeof p === "string" ? p : p?.url;
      if (!/^https?:\/\//.test(u ?? "")) bad.push(`draws.providers: ${redactUrl(u)} needs an http(s) url (and an optional span, the widest eth_getLogs block range it serves)`);
      if (typeof p === "object" && p?.span !== undefined && !(Number.isSafeInteger(p.span) && p.span >= 1)) bad.push(`draws.providers: ${redactUrl(u)}: span is a whole number of blocks`);
    }
    if ((d.providers ?? evm.rpcs ?? []).length < 2) bad.push("draws.providers: at least two providers (each keeps its own holder index; a root is verified by two)");
    if (d.verify_quorum !== undefined && !(Number.isSafeInteger(d.verify_quorum) && d.verify_quorum >= 1)) bad.push("draws.verify_quorum: a whole number, at least 1 (default 2)");
    if (d.deadline_s !== undefined && !(Number.isSafeInteger(d.deadline_s) && d.deadline_s >= 120 && d.deadline_s < 900)) bad.push("draws.deadline_s: seconds before the pin, 120 to 899 (default 360): leave the guardian or the Safe time to act inside the 15-minute window");
    for (const k of ["pause_unverified", "pause_on_degraded", "reconcile"]) if (d[k] !== undefined && typeof d[k] !== "boolean") bad.push(`draws.${k}: true or false`);
  }
  if (cfg.evm?.guardian && !cfg.evm) bad.push("evm.guardian without evm");
  // docs/v7/cycles-from-fees.md §4.2: the fueler's ops key, its /status, the providers its nonces are read from
  if (cfg.fueler !== undefined) {
    const f = cfg.fueler ?? {};
    if (!/^0x[0-9a-fA-F]{40}$/.test(f.ops ?? "")) bad.push("fueler.ops: the fueler's address (treasury.ops())");
    if (!/^https?:\/\//.test(f.status_url ?? "")) bad.push("fueler.status_url: the fueler's /status (http(s))");
    // docs/v9/fueler-solana.md §6: with sol_ops (the Solana hot key = the canister's treasury.ops) it watches the books home: rpcs_sol, not rpcs_4663
    const solHome = f.sol_ops !== undefined;
    if (solHome && !/^[1-9A-HJ-NP-Za-km-z]{32,44}$/.test(f.sol_ops ?? "")) bad.push("fueler.sol_ops: the fueler's Solana hot key (base58; the canister's treasury.ops)");
    for (const k of solHome ? ["rpcs_sol", "rpcs_1"] : ["rpcs_4663", "rpcs_1"]) if (!Array.isArray(f[k]) || f[k].length < 2 || f[k].some((u) => !/^https?:\/\//.test(u ?? ""))) bad.push(`fueler.${k}: two http(s) providers at least (any one that sees a foreign transaction pages)`);
    if (f.min_ops_eth !== undefined && !(typeof f.min_ops_eth === "number" && f.min_ops_eth >= 0 && f.min_ops_eth <= 10)) bad.push("fueler.min_ops_eth: ETH, 0 to 10");
    if (f.min_sol !== undefined && !(typeof f.min_sol === "number" && f.min_sol >= 0 && f.min_sol <= 100)) bad.push("fueler.min_sol: SOL, 0 to 100");
    if (f.trigger_t !== undefined && !(typeof f.trigger_t === "number" && f.trigger_t > 0 && f.trigger_t <= 10_000)) bad.push("fueler.trigger_t: the fueler's FUELER_BUYER_LOW_T, in T");
    if (f.stuck_h !== undefined && !(typeof f.stuck_h === "number" && f.stuck_h >= 1 && f.stuck_h <= 336)) bad.push("fueler.stuck_h: hours under the trigger with no conversion before it pages, 1 to 336");
    for (const k of Object.keys(f)) if (!["ops", "status_url", "rpcs_4663", "rpcs_1", "min_ops_eth", "sol_ops", "rpcs_sol", "min_sol", "trigger_t", "stuck_h"].includes(k) && !k.startsWith("//")) bad.push(`fueler.${k}: unknown`);
  }
  for (const l of cfg.liveness ?? []) {
    if (!l?.name || !/^https?:\/\//.test(l?.url ?? "")) bad.push(`liveness: ${JSON.stringify(l)} needs a name and an http(s) url (a service's /healthz)`);
  }
  // docs/v7/phase1-nft.md §6: the Ethereum NFT side
  // go-live (docs/v7/golive-runbook.md §6): the burn-rate page's thresholds (watcher.mjs BURN_DEFAULTS)
  if (cfg.cycles !== undefined) {
    const c = cfg.cycles, num = (k, lo, hi) => c[k] === undefined || (typeof c[k] === "number" && c[k] >= lo && c[k] <= hi);
    if (typeof c !== "object" || c === null || Array.isArray(c)) bad.push("cycles: an object of thresholds (window_h, min_days, min_days_sev1, burn_warn_t_per_day, min_balance_t)");
    else {
      const known = ["window_h", "min_days", "min_days_sev1", "burn_warn_t_per_day", "min_balance_t"];
      for (const k of Object.keys(c)) if (!known.includes(k) && !k.startsWith("//")) bad.push(`cycles.${k}: unknown (${known.join(", ")})`);
      if (!num("window_h", 6, 168)) bad.push("cycles.window_h: hours the burn rate is measured over, 6 to 168");
      if (!num("min_days", 1, 365)) bad.push("cycles.min_days: days of cycles left under which it pages SEV2, 1 to 365");
      if (!num("min_days_sev1", 0.25, 365)) bad.push("cycles.min_days_sev1: days left under which it pages SEV1, 0.25 to 365");
      if (!num("burn_warn_t_per_day", 1, 10_000)) bad.push("cycles.burn_warn_t_per_day: T cycles a day above which the rate pages, 1 to 10,000");
      if (!num("min_balance_t", 0, 100_000)) bad.push("cycles.min_balance_t: T cycles under which it pages whatever the rate, 0 to 100,000");
      if (typeof c.min_days === "number" && typeof c.min_days_sev1 === "number" && c.min_days_sev1 > c.min_days) bad.push("cycles.min_days_sev1 must not exceed cycles.min_days");
    }
  }
  if (cfg.eth_nft) {
    const e = cfg.eth_nft, A = /^0x[0-9a-fA-F]{40}$/;
    if (!(e.home_rpc ?? []).length) bad.push("eth_nft.home_rpc: a 4663 RPC (the wrappers and modules are read there)");
    if ((e.eth_rpc ?? []).length < 2 || new Set(e.eth_rpc).size !== e.eth_rpc.length) bad.push("eth_nft.eth_rpc: two distinct Ethereum providers at least (custody is read on both)");
    if (!(e.collections ?? []).length) bad.push("eth_nft.collections: at least one");
    for (const c of e.collections ?? []) {
      for (const k of ["collection", "wrapper", "module", "custodian"]) if (!A.test(c?.[k] ?? "")) bad.push(`eth_nft.collections ${c?.slug ?? "?"}: ${k} is not an address`);
      if (!c?.slug) bad.push("eth_nft.collections: each needs a slug");
    }
    // D8 (the operator, 2026-09-27): the guardian pauses the keeper — zeroing the canister's poster — when the posted floor is more
    // than 20 % from both public references. So an eth_nft monitor has an armed guardian unless the config says, in words, it does not.
    if (e.guardian_pause !== false && (!cfg.evm?.guardian || cfg.evm.guardian.auto_pause === false))
      bad.push("eth_nft: D8 needs evm.guardian with auto_pause (the floor pause at 20 % from both references); set eth_nft.guardian_pause = false only for a rehearsal");
    if (e.guardian_pause !== undefined && typeof e.guardian_pause !== "boolean") bad.push("eth_nft.guardian_pause: true or false");
    // round 2 (the review's launch-configuration note): with D8 armed, a collection whose floor goes unposted has its poster paused
    // after evm.guardian.pause_on_no_good_post_h, so its holders burn at the frozen floor instead of waiting for the 7-day escape
    if (e.guardian_pause !== false && cfg.evm?.guardian && cfg.evm.guardian.pause_on_no_good_post_h === undefined)
      bad.push("eth_nft: evm.guardian.pause_on_no_good_post_h must be armed (2 to 160 hours; the launch template says 6): without it a collection whose floor goes unposted refuses burns until the 7-day escape");
    if (e.deposits_scan !== undefined && typeof e.deposits_scan !== "boolean") bad.push("eth_nft.deposits_scan: true or false");
    // the operator's Dwellir plan (2026-09-27) serves eth_getLogs over at most 500 blocks a request
    if (e.logs_max_blocks !== undefined && !(Number.isInteger(e.logs_max_blocks) && e.logs_max_blocks >= 10 && e.logs_max_blocks <= 10_000))
      bad.push("eth_nft.logs_max_blocks: the widest eth_getLogs range, 10 to 10,000 blocks (500 on the Dwellir plan)");
    // audit 2026-09-27-nft D-M-1: D8 pauses on two references that agree, so each collection names both (OpenSea and CoinGecko)
    // The operator, 2026-10-09 (deployments/launch/nft-pilot.json `priceRefs`): a collection CoinGecko does not list may name OpenSea as
    // its only reference, price_refs ["opensea"], and only where the launch file (when it is loaded) decides exactly that for it; then it
    // names no coingecko_id (one source of truth for which references the pause reads). Every other collection still names both
    const LR = (cfg.launch?.collections ?? []);
    for (const c of e.collections ?? []) {
      const who = `eth_nft.collections ${c?.slug ?? "?"}`;
      if (c?.price_refs !== undefined) {
        if (!(Array.isArray(c.price_refs) && c.price_refs.length === 1 && c.price_refs[0] === "opensea"))
          bad.push(`${who}: price_refs is either absent (both references) or exactly ["opensea"] (OpenSea alone, the operator's per-collection decision)`);
        else {
          if (!c.opensea_slug) bad.push(`${who}: price_refs ["opensea"] needs opensea_slug`);
          if (c.coingecko_id) bad.push(`${who}: price_refs ["opensea"] reads OpenSea alone: remove coingecko_id (or price_refs)`);
          if (cfg.launch) {
            const l = LR.find((x) => String(x.collection ?? "").toLowerCase() === String(c.collection ?? "").toLowerCase());
            const lr = l?.priceRefs;
            if (!(Array.isArray(lr) && lr.length === 1 && lr[0] === "opensea"))
              bad.push(`${who}: price_refs ["opensea"] where the launch file (${cfg.launch_file ?? "launch"}) does not decide priceRefs ["opensea"] for this collection: D8 needs both references`);
          }
        }
      } else if (e.guardian_pause !== false && (!c?.opensea_slug || !c?.coingecko_id))
        bad.push(`${who}: D8's pause needs both references (opensea_slug and coingecko_id)`);
    }
    for (const [m, t] of Object.entries(e.floor_pause_ack ?? {})) if (!A.test(m) || !Number.isSafeInteger(t))
      bad.push(`eth_nft.floor_pause_ack: ${m}: a module address → the epoch second the operator acknowledged its floor pause`);
  }
  if (bad.length) throw new Error(`the configuration is refused:\n- ${bad.join("\n- ")}`);
  return cfg;
}
