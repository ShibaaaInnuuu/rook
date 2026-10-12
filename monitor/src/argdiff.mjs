// The upgrade-argument diff and the words for a proposal (docs/v6/k6-watcher-canary.md §2). Pure: no network, no clock.
//
// Every comparison is between EFFECTIVE values: `custody_caps = null` is stage 0 for the wrapper's inscriptions and
// `outflow_limits = null` is `Limits::stage0` (autobuyer/canister-rs/src/canister.rs `caps_of` / `limits_of`), so writing the
// stage-0 numbers out is not a change, and a raise from null is measured from stage 0.

// autobuyer/canister-rs/src/custodycap.rs FIRST_* and outflow.rs Limits::stage0: mirrored here, and checked against the
// canister's own encoding of an explicit stage 0 in the tests.
import { redactUrl } from "./config.mjs";

const TEN_ETH = 10_000_000_000_000_000_000n;
const FIFTEEN_ETH = 15_000_000_000_000_000_000n;
export const STAGE0_CAP = { max_items: 3, max_value_wei: TEN_ETH, max_item_wei: TEN_ETH, global_max_items: 6, global_max_value_wei: TEN_ETH };
// J7 (the operator, 2026-09-27): an Ethereum collection (`<chain>:0x…`) starts at 15 ETH (one item up to 15 ETH), 6 items by default
// and up to 24 when the argument names it; the global cap from nothing is stage 0's or the sum of the collections' caps, if more
export const STAGE0_EVM_CAP = { max_items: 6, first_max_items: 24, max_value_wei: FIFTEEN_ETH, max_item_wei: FIFTEEN_ETH };
// H3 (the operator, 2026-09-27): 25 ETH of value a day at stage 0 (10 ETH before)
export const STAGE0_LIMITS = { per_collection_items: 3, global_items: 6, per_collection_mints: 3, global_mints: 6,
  global_value_wei: 25_000_000_000_000_000_000n, trip_factor: 3 };
const isEvm = (key) => !String(key).startsWith("btc:");
/** `custodycap::first_of`: the most a collection new to the policy may start at. */
const firstOf = (key) => isEvm(key) ? { items: BigInt(STAGE0_EVM_CAP.first_max_items), value: FIFTEEN_ETH, item: FIFTEEN_ETH }
  : { items: BigInt(STAGE0_CAP.max_items), value: TEN_ETH, item: TEN_ETH };
/** `custodycap::first_global`: stage 0's global cap, or the sum of the collections' caps when that is more. */
const firstGlobal = (cols) => {
  const items = cols.reduce((a, c) => a + BigInt(c.max_items), 0n), value = cols.reduce((a, c) => a + big(c.max_value_wei), 0n);
  return { items: items > 6n ? items : 6n, value: value > TEN_ETH ? value : TEN_ETH };
};
const MAX_RAISE_FACTOR = 4n;

const opt = (x) => (Array.isArray(x) ? (x.length ? x[0] : null) : x ?? null);
const big = (x) => BigInt(String(x).replace(/_/g, ""));

/** The wei amount in ETH, as a person reads it: 10 ETH, 0.65 ETH. */
export function eth(wei) {
  const w = big(wei), whole = w / 10n ** 18n, frac = w % 10n ** 18n;
  if (frac === 0n) return `${whole} ETH`;
  return `${whole}.${frac.toString().padStart(18, "0").replace(/0+$/, "").slice(0, 6)} ETH`;
}

/** `canister.rs` `caps_of`: the argument's caps, or stage 0 for the wrapper's inscriptions (`btc:0x<wrapper>`, lower-case). */
export function effectiveCaps(init) {
  const c = opt(init.custody_caps);
  if (c) return { collections: c.collections.map((x) => ({ collection: x.collection, max_items: Number(x.max_items), max_value_wei: big(x.max_value_wei),
    max_item_wei: big(x.max_item_wei) })), global_max_items: Number(c.global_max_items), global_max_value_wei: big(c.global_max_value_wei) };
  const w = opt(init.wrapper);
  const listed = w && /^0x[0-9a-fA-F]{40}$/.test(w) ? [`btc:0x${w.slice(2).toLowerCase()}`] : [];
  // each eth_nft collection at stage 0 beside the wrapper's inscriptions (canister.rs caps_of; J7's numbers)
  const e = opt(init.eth_nft);
  for (const c of e?.collections ?? []) listed.push(`${e.chain_id}:${String(c.collection).toLowerCase()}`);
  const collections = listed.map((collection) => isEvm(collection)
    ? { collection, max_items: STAGE0_EVM_CAP.max_items, max_value_wei: STAGE0_EVM_CAP.max_value_wei, max_item_wei: STAGE0_EVM_CAP.max_item_wei }
    : { collection, max_items: STAGE0_CAP.max_items, max_value_wei: STAGE0_CAP.max_value_wei, max_item_wei: STAGE0_CAP.max_item_wei });
  const g = firstGlobal(collections);
  return { collections, global_max_items: Number(g.items), global_max_value_wei: g.value };
}

/** `canister.rs` `limits_of`. */
export function effectiveLimits(init) {
  const l = opt(init.outflow_limits);
  if (!l) return { ...STAGE0_LIMITS };
  return { per_collection_items: Number(l.per_collection_items), global_items: Number(l.global_items),
    per_collection_mints: Number(l.per_collection_mints), global_mints: Number(l.global_mints), global_value_wei: big(l.global_value_wei),
    trip_factor: Number(l.trip_factor) };
}

const LIMIT_WORDS = {
  per_collection_items: "items out a day, per collection", global_items: "items out a day, in all",
  per_collection_mints: "mints a day, per collection", global_mints: "mints a day, in all", global_value_wei: "value out a day",
  trip_factor: "tripwire factor (acts before custody freezes, × the limit)",
};
const CAP_WORDS = { max_items: "items", max_value_wei: "value", max_item_wei: "value of one item" };
const IDENTITY = ["chain_id", "vault", "key_name", "accept_production_key_name", "bridge_allowlist", "btc_network", "wrapper"];
const show = (k, v) => (/_wei$/.test(k) ? eth(v) : String(v));
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const list = (x) => (Array.isArray(x) && x.length ? x.join(", ") : "(none)");

/** A raise past ×4 (or a new cap above stage 0) that `custodycap::validate_change` refuses in post_upgrade. */
const refusedRaise = (old, now, first) => (old === 0n ? now > first : now > old * MAX_RAISE_FACTOR);

const USABLE_DAYS = 2n;   // canister.rs ASSUMED_DELAY_SECS (72 h) less a day (outflow::exit_fits)
/**
 * Review round 5 (A4-L-1's rule on a proposal): the exit rule canister.rs validate_custody holds an install or upgrade to, on caps
 * `{key: {items}}`, a global item cap and limits: every collection's items, and all of them (the global cap where it binds), must leave
 * within the timelock's 72 h less a day. [] when they can.
 */
export function exitProblemsOf(caps, globalItems, limits) {
  const out = [];
  const per = BigInt(limits.per_collection_items), glob = BigInt(limits.global_items);
  for (const [k, c] of Object.entries(caps)) if (per === 0n || (BigInt(c.items) + per - 1n) / per > USABLE_DAYS) out.push(`${k} holds up to ${c.items} items at ${per} a day`);
  const sum = Object.values(caps).reduce((a, c) => a + BigInt(c.items), 0n), g = BigInt(globalItems);
  const total = sum < g ? sum : g;
  if (glob === 0n || (total + glob - 1n) / glob > USABLE_DAYS) out.push(`custody holds up to ${total} items in all at ${glob} a day in all`);
  return out;
}

/** The exit rule on an Init's effective caps and limits. */
export function exitProblemsOfInit(init) {
  const c = effectiveCaps(init);
  return exitProblemsOf(Object.fromEntries(c.collections.map((x) => [x.collection, { items: x.max_items }])), c.global_max_items, effectiveLimits(init));
}

/**
 * The change from `base` to `next` (both plain Init), as lines. Each line: { sev: CRITICAL | WARN | INFO, verb: LOOSENS |
 * TIGHTENS | CHANGES, text }. Also `unchanged`: the fields that did not move. Loosening first.
 */
export function diffInit(base, next) {
  const lines = [], unchanged = [];
  const add = (sev, verb, text, tag = null) => lines.push({ sev, verb, text, tag });

  for (const f of IDENTITY) {
    if (same(base[f], next[f])) { unchanged.push(f); continue; }
    add("CRITICAL", "CHANGES", `${f}: ${JSON.stringify(opt(base[f]) ?? null)} → ${JSON.stringify(opt(next[f]) ?? null)} (an identity field: the running code keeps the installed value on upgrade, a new module may not; it should never differ)`);
  }
  if (same(base.operator, next.operator)) unchanged.push("operator");
  else add("CRITICAL", "CHANGES", `operator: ${opt(base.operator) ?? "(none)"} → ${opt(next.operator) ?? "(none)"} (whoever it names can call every operator path)`);

  // docs/v9/solana-migration.md §4: the home chain. Its identity (cluster, rook_core, nonce_seed) is set once from nothing and kept by the
  // running code on upgrade (`restore_home`); its providers and the lifetime switch are operational
  const ha = opt(base.home)?.Solana ?? null, hb = opt(next.home)?.Solana ?? null;
  if (same(ha, hb)) unchanged.push("home");
  else if (!ha && hb) add("CRITICAL", "CHANGES", `home: the EVM chain → Solana ${hb.cluster} (rook_core ${hb.rook_core}, nonce seed ${JSON.stringify(hb.nonce_seed)}; the Ethereum NFT side's home steps are then signed by the canister's Ed25519 key on Solana)`);
  else if (ha && !hb) add("CRITICAL", "CHANGES", "home: Solana → (none) (the running code keeps the installed home on upgrade; a new module may not)");
  else {
    for (const f of ["cluster", "rook_core", "nonce_seed"]) {
      if (!same(ha[f], hb[f])) add("CRITICAL", "CHANGES", `home.${f}: ${JSON.stringify(ha[f])} → ${JSON.stringify(hb[f])} (an identity field: the running code keeps the installed value on upgrade, a new module may not; it should never differ)`);
    }
    if (!same(ha.rpc_urls, hb.rpc_urls)) add("WARN", "CHANGES", `home.rpc_urls: ${JSON.stringify(ha.rpc_urls)} → ${JSON.stringify(hb.rpc_urls)} (every Solana read must agree across these)`);
    // audit A6-13: rook_backing's program id is identity once set (an install without it takes the argument's once)
    const rb = (x) => opt(x.rook_backing) ?? null;
    if (rb(ha) !== rb(hb)) add(rb(ha) === null ? "WARN" : "CRITICAL", "CHANGES", `home.rook_backing: ${rb(ha) ?? "(none: the dev id)"} → ${rb(hb) ?? "(none)"} (${rb(ha) === null ? "set once, from nothing" : "an identity field: the running code keeps the installed value"})`);
    // decision D-i: the drand relays the registry keeper asks. Operational: the canister verifies every round's BLS signature itself, so a
    // relay can stall a proof (liveness) but never choose its randomness
    const dr = (x) => opt(x.drand_urls) ?? null;
    if (!same(dr(ha), dr(hb))) add("WARN", "CHANGES", `home.drand_urls: ${JSON.stringify(dr(ha) ?? "(defaults)")} → ${JSON.stringify(dr(hb) ?? "(defaults)")} (two must serve the same signature of a round, which the canister also verifies under evmnet's key)`);
    const fb = (x) => opt(x.blockhash_fallback) === true;
    if (fb(ha) !== fb(hb)) add("WARN", "CHANGES", `home.blockhash_fallback: ${fb(ha) ? "on" : "off"} → ${fb(hb) ? "on" : "off"} (${fb(hb) ? "recent blockhashes, re-signed on proven expiry" : "the durable nonce"})`);
  }

  // audit D, H-2: holder deposits are off unless `true`; turning them on wakes the deposits pass, its returns and their flow
  const hd = (x) => opt(x) === true;
  if (hd(base.holder_deposits) === hd(next.holder_deposits)) unchanged.push("holder_deposits");
  else if (hd(next.holder_deposits)) add("CRITICAL", "LOOSENS", "holder_deposits: off → ON (the canister starts classifying, minting for and returning strangers' deposits; needs a paged deposit discovery first, fixes-D-canister.md §7)");
  else add("INFO", "TIGHTENS", "holder_deposits: on → off");
  // audit D, H-1: where the ticket-log scanner starts when it first runs and is not idle
  const wf = (x) => (opt(x) === null ? null : String(opt(x)));
  if (wf(base.wrapper_from_block) === wf(next.wrapper_from_block)) unchanged.push("wrapper_from_block");
  else add("WARN", "CHANGES", `wrapper_from_block: ${wf(base.wrapper_from_block) ?? "null"} → ${wf(next.wrapper_from_block) ?? "null"} (where the ticket-log scanner first reads; only used before its first read)`);

  // docs/v6/relay-conversion.md §3.5: null = Relay at 300 bps, no float. A higher fee bound or float_first on loosens.
  const ff = (x) => { const v = opt(x); return v === null ? { max_fee_bps: 300, float_first: false, set: false } : { max_fee_bps: Number(v.max_fee_bps), float_first: v.float_first === true, set: true }; };
  const fa = ff(base.frog_funding), fb = ff(next.frog_funding);
  if (fa.max_fee_bps === fb.max_fee_bps && fa.float_first === fb.float_first) unchanged.push("frog_funding");
  else {
    if (fb.max_fee_bps > fa.max_fee_bps) add(fb.max_fee_bps > 500 ? "CRITICAL" : "WARN", "LOOSENS", `frog_funding.max_fee_bps: ${fa.max_fee_bps} → ${fb.max_fee_bps} (Relay may keep up to ${fb.max_fee_bps / 100}% of each advance${fb.max_fee_bps > 500 ? "; over the 500 ceiling: the canister refuses it at install" : ""})`);
    else if (fb.max_fee_bps < fa.max_fee_bps) add("INFO", "TIGHTENS", `frog_funding.max_fee_bps: ${fa.max_fee_bps} → ${fb.max_fee_bps}`);
    if (fb.float_first && !fa.float_first) add("WARN", "LOOSENS", "frog_funding.float_first: off → ON (a frog is bought from an operator float when it covers the cap, and reimbursed by itemArrived)");
    else if (!fb.float_first && fa.float_first) add("INFO", "TIGHTENS", "frog_funding.float_first: on → off (every frog is paid by its own advance through Relay)");
  }

  // docs/v7/phase1-nft.md §4.1: the Ethereum NFT side. A collection added, deposits on, a wider Relay or premium bound loosen.
  const en = (x) => opt(x);
  const na = en(base.eth_nft), nb = en(next.eth_nft);
  if (same(na, nb)) unchanged.push("eth_nft");
  else if (!na && nb) add("CRITICAL", "LOOSENS", `eth_nft: off → ON (chain ${nb.chain_id}; ${nb.collections.length} collection(s): ${nb.collections.map((c) => c.collection).join(", ")}; the canister buys, holds and delivers Ethereum NFTs and posts their modules' floors)`);
  else if (na && !nb) add("WARN", "TIGHTENS", "eth_nft: on → off (nothing new is bought or delivered on Ethereum; NFTs held stay at the canister's address)");
  else {
    const key = (c) => String(c.collection).toLowerCase();
    const ca = new Map(na.collections.map((c) => [key(c), c])), cb = new Map(nb.collections.map((c) => [key(c), c]));
    for (const [k, c] of cb) {
      const o = ca.get(k);
      const chainOf = (x, cfg) => String(opt(x.chain_id) ?? cfg.chain_id);
      const tb = (x) => opt(x.token_bound) === true;
      if (!o) add("CRITICAL", "LOOSENS", `eth_nft: adds collection ${k} on chain ${chainOf(c, nb)} (wrapper ${c.wrapper}, module ${c.module}, up to ${c.max_purchase_wei} wei a purchase${tb(c) ? "; TOKEN-BOUND: each NFT is an account whose contents its seller can change, priced and moved as the NFT alone" : ""})`);
      else {
        if (chainOf(o, na) !== chainOf(c, nb)) add("CRITICAL", "CHANGES", `eth_nft ${k}: chain ${chainOf(o, na)} → ${chainOf(c, nb)}`);
        if (tb(o) !== tb(c)) add("CRITICAL", "CHANGES", `eth_nft ${k}: token_bound ${tb(o)} → ${tb(c)}`);
        // audit C3-01: a token-bound collection's NFT-alone share of a price: raising it pays more for what an account may no longer hold
        const nv = (x) => Number(opt(x.nft_value_bps) ?? 10_000);
        if (nv(c) > nv(o)) add("CRITICAL", "LOOSENS", `eth_nft ${k}: nft_value_bps ${nv(o)} → ${nv(c)} (more of a token-bound account's price is paid for the NFT)`);
        else if (nv(c) < nv(o)) add("WARN", "TIGHTENS", `eth_nft ${k}: nft_value_bps ${nv(o)} → ${nv(c)}`);
        if (o.wrapper !== c.wrapper || o.module !== c.module) add("CRITICAL", "CHANGES", `eth_nft ${k}: wrapper/module ${o.wrapper}/${o.module} → ${c.wrapper}/${c.module}`);
        if (BigInt(c.max_purchase_wei) > BigInt(o.max_purchase_wei)) add("WARN", "LOOSENS", `eth_nft ${k}: max_purchase_wei ${o.max_purchase_wei} → ${c.max_purchase_wei}`);
        if (Number(c.max_premium_bps) > Number(o.max_premium_bps)) add("WARN", "LOOSENS", `eth_nft ${k}: max_premium_bps ${o.max_premium_bps} → ${c.max_premium_bps} (a buy may pay more over our floor)`);
      }
    }
    for (const k of ca.keys()) if (!cb.has(k)) add("WARN", "CHANGES", `eth_nft: drops collection ${k} (its NFTs stay held; nothing new is bought or delivered for it)`);
    if (Number(nb.relay_max_fee_bps) > Number(na.relay_max_fee_bps)) add("WARN", "LOOSENS", `eth_nft.relay_max_fee_bps: ${na.relay_max_fee_bps} → ${nb.relay_max_fee_bps}`);
    if (nb.deposits && !na.deposits) add("CRITICAL", "LOOSENS", "eth_nft.deposits: off → ON (holders' NFTs sent with a claim are minted on the canister's word; NFTs of a configured collection sent without one go back to their sender)");
    else if (!nb.deposits && na.deposits) add("INFO", "TIGHTENS", "eth_nft.deposits: on → off");
    if (!same(na.floor, nb.floor)) add("WARN", "CHANGES", `eth_nft.floor: ${JSON.stringify(opt(na.floor) ?? "defaults")} → ${JSON.stringify(opt(nb.floor) ?? "defaults")} (how the canister reads the floor it posts)`);
    if (String(na.chain_id) !== String(nb.chain_id)) add("CRITICAL", "CHANGES", `eth_nft.chain_id: ${na.chain_id} → ${nb.chain_id}`);
    // A2-L-1 (2026-09-27): the gas pot's target; above it the collections' leftover goes back to their modules
    const gp = (x) => BigInt(opt(x?.gas_pot_target_wei) ?? 50_000_000_000_000_000n);
    if (gp(nb) !== gp(na)) add(gp(nb) > gp(na) ? "WARN" : "INFO", gp(nb) > gp(na) ? "LOOSENS" : "TIGHTENS",
      `eth_nft.gas_pot_target_wei: ${eth(gp(na))} → ${eth(gp(nb))} (what stays at the canister's Ethereum address before leftover is swept back)`);
    const lm = (x) => Number(opt(x?.logs_max_blocks) ?? 500);
    if (lm(nb) !== lm(na)) add("INFO", "CHANGES", `eth_nft.logs_max_blocks: ${lm(na)} → ${lm(nb)} (the widest eth_getLogs range the sale scan asks)`);
    // docs/v7/dormant-collections.md §3.2: the dormancy's caps are the cycles budget and the griefing bound; the desk is the public path
    const da = opt(na.dormancy), db = opt(nb.dormancy);
    if (!same(da, db)) {
      if (!da && db) add("WARN", "CHANGES", `eth_nft.dormancy: off → ON (collections sleep until a wake: max_active ${db.max_active}, max_waking ${db.max_waking}, ${db.wakes_per_day} wakes a day, desk ${opt(db.wake_desk) ?? "none"})`);
      else if (da && !db) add("WARN", "CHANGES", "eth_nft.dormancy: on → off (every collection is watched all the time: the cycles burn scales with every listed collection)");
      else {
        for (const f of ["max_active", "max_waking", "wakes_per_day"]) {
          if (Number(db[f]) > Number(da[f])) add("WARN", "LOOSENS", `eth_nft.dormancy.${f}: ${da[f]} → ${db[f]} (more collections awake or woken: more cycles a day)`);
          else if (Number(db[f]) < Number(da[f])) add("INFO", "TIGHTENS", `eth_nft.dormancy.${f}: ${da[f]} → ${db[f]}`);
        }
        for (const f of ["wake_timeout_secs", "cooldown_secs", "idle_days"]) {
          if (String(db[f]) !== String(da[f])) add("INFO", "CHANGES", `eth_nft.dormancy.${f}: ${da[f]} → ${db[f]}`);
        }
        // §11: the scaling from the cycles balance: off means the ceilings are fixed caps; a lower burn estimate, runway or reserve lets
        // more collections wake on the same balance
        const sa = opt(da.scaling), sb = opt(db.scaling);
        if (!sa && sb) add("WARN", "CHANGES", `eth_nft.dormancy.scaling: off → ON (the limits follow the cycles balance under max_active ${db.max_active} and ${db.wakes_per_day} wakes a day: ${Number(sb.active_b_per_day) / 1000} T a day per awake collection, ${sb.runway_days} days of runway, ${Number(sb.reserve_b) / 1000} T reserve)`);
        else if (sa && !sb) add("WARN", "CHANGES", `eth_nft.dormancy.scaling: on → off (max_active ${db.max_active} and ${db.wakes_per_day} wakes a day are fixed caps, whatever the balance)`);
        else if (sa && sb) {
          for (const f of ["active_b_per_day", "idle_b_per_day", "wake_b", "runway_days", "reserve_b", "exit_b_per_awake"]) {
            if (Number(sb[f]) < Number(sa[f])) add("WARN", "LOOSENS", `eth_nft.dormancy.scaling.${f}: ${sa[f]} → ${sb[f]} (more collections awake or woken on the same cycles balance)`);
            else if (Number(sb[f]) > Number(sa[f])) add("INFO", "TIGHTENS", `eth_nft.dormancy.scaling.${f}: ${sa[f]} → ${sb[f]}`);
          }
        }
        const dk = (x) => String(opt(x.wake_desk) ?? "none").toLowerCase();
        if (dk(da) !== dk(db)) add("WARN", "CHANGES", `eth_nft.dormancy.wake_desk: ${dk(da)} → ${dk(db)} (where the canister reads the public, bonded wake requests)`);
      }
    }
  }

  for (const f of ["rpc_urls", "btc_api_urls"]) {
    const a = base[f] ?? [], b = next[f] ?? [];
    if (same([...a].sort(), [...b].sort())) { unchanged.push(f); continue; }
    // compared in full (a key change is a change), printed as scheme://host[:port] only (bayc10-decisions.md §5: a key lives in the
    // path, query, fragment or userinfo, and this text goes to Telegram): a provider whose printed form is unchanged is "reconfigured"
    let added = b.filter((x) => !a.includes(x)), removed = a.filter((x) => !b.includes(x));
    const reconf = [];
    for (const x of [...added]) {
      const j = removed.findIndex((y) => redactUrl(y) === redactUrl(x));
      if (j < 0) continue;
      reconf.push(redactUrl(x));
      removed = removed.filter((_, i) => i !== j);
      added = added.filter((y) => y !== x);
    }
    const what = f === "rpc_urls" ? "4663 RPC providers" : "Bitcoin providers";
    if (b.length < a.length) add("CRITICAL", "LOOSENS", `${what}: ${a.length} → ${b.length}; fewer operators need to lie together`);
    add("CRITICAL", "CHANGES", `${what}${added.length ? `; adds ${added.map(redactUrl).join(", ")}` : ""}${removed.length ? `; removes ${removed.map(redactUrl).join(", ")}` : ""}${reconf.length ? `; ${reconf.join(", ")} reconfigured (a path or key changed)` : ""} (the providers are what the canister believes)`);
  }

  const ea = opt(base.evm_chains) ?? [], eb = opt(next.evm_chains) ?? [];
  if (same(ea, eb)) unchanged.push("evm_chains");
  else {
    const ids = (v) => v.map((c) => String(c.chain_id));
    const added = ids(eb).filter((x) => !ids(ea).includes(x)), removed = ids(ea).filter((x) => !ids(eb).includes(x));
    const changed = ids(eb).filter((x) => ids(ea).includes(x) && !same(ea.find((c) => String(c.chain_id) === x), eb.find((c) => String(c.chain_id) === x)));
    if (added.length) add("CRITICAL", "LOOSENS", `evm_chains: adds chain ${added.join(", ")} (a new chain the canister may buy and sign on)`);
    if (removed.length) add("INFO", "TIGHTENS", `evm_chains: removes chain ${removed.join(", ")}`);
    if (changed.length) add("CRITICAL", "CHANGES", `evm_chains: chain ${changed.join(", ")} reconfigured (providers, Seaport, fees or zones)`);
  }

  // custody caps, per collection and global
  const ca = effectiveCaps(base), cb = effectiveCaps(next);
  let capsMoved = false;
  for (const c of cb.collections) {
    const o = ca.collections.find((x) => x.collection === c.collection);
    if (!o) {
      capsMoved = true;
      const f = firstOf(c.collection);
      const refused = BigInt(c.max_items) > f.items || c.max_value_wei > f.value || c.max_item_wei > f.item;
      add("CRITICAL", "LOOSENS", `custody cap: NEW collection ${c.collection}: ${c.max_items} items, ${eth(c.max_value_wei)}${refused ? " (above stage 0 for a new collection: the running code refuses it, so executing it fails)" : ""}`, `new:${c.collection}`);
      continue;
    }
    for (const k of ["max_items", "max_value_wei", "max_item_wei"]) {
      const x = big(o[k]), y = big(c[k]);
      if (x === y) continue;
      capsMoved = true;
      const f = firstOf(c.collection);
      const refused = y > x && refusedRaise(x, y, k === "max_items" ? f.items : k === "max_value_wei" ? f.value : f.item);
      add(y > x ? "CRITICAL" : "INFO", y > x ? "LOOSENS" : "TIGHTENS",
        `custody cap ${c.collection}: ${CAP_WORDS[k]} ${show(k, x)} → ${show(k, y)}${refused ? " (more than ×4 at once: the running code refuses it, so executing it fails)" : ""}`, `cap:${c.collection}:${k}`);
    }
  }
  for (const o of ca.collections) {
    if (!cb.collections.some((c) => c.collection === o.collection)) { capsMoved = true; add("INFO", "TIGHTENS", `custody cap: removes ${o.collection} (it becomes unlisted: every acquisition refused)`); }
  }
  const fg = firstGlobal(cb.collections);
  for (const [k, words, first] of [["global_max_items", "items in all", fg.items], ["global_max_value_wei", "value in all", fg.value]]) {
    const x = big(ca[k]), y = big(cb[k]);
    if (x === y) continue;
    capsMoved = true;
    const refused = y > x && refusedRaise(x, y, first);
    add(y > x ? "CRITICAL" : "INFO", y > x ? "LOOSENS" : "TIGHTENS",
      `custody cap, global: ${words} ${k.endsWith("_wei") ? eth(x) : x} → ${k.endsWith("_wei") ? eth(y) : y}${refused ? " (more than ×4 at once: refused by the running code)" : ""}`, `global:${k}`);
  }
  if (!capsMoved) unchanged.push("custody_caps");

  const la = effectiveLimits(base), lb = effectiveLimits(next);
  let limitsMoved = false;
  for (const k of Object.keys(LIMIT_WORDS)) {
    const x = big(la[k]), y = big(lb[k]);
    if (x === y) continue;
    limitsMoved = true;
    add(y > x ? "CRITICAL" : "INFO", y > x ? "LOOSENS" : "TIGHTENS", `outflow limit: ${LIMIT_WORDS[k]} ${show(k, x)} → ${show(k, y)}`, `limit:${k}`);
  }
  if (!limitsMoved) unchanged.push("outflow_limits");

  const rank = { LOOSENS: 0, CHANGES: 1, TIGHTENS: 2 };
  lines.sort((a, b) => rank[a.verb] - rank[b.verb]);
  return { lines, unchanged };
}

// ---- the published reproducible builds -------------------------------------------------------------------------------------
/** `builds`: [{ commit, buyer, timelock }] as `deployments/canister-ic.json` records them. Which build, if any, has this hash. */
export function buildOf(builds, which, hash) {
  const h = String(hash ?? "").toLowerCase().replace(/^0x/, "");
  return (builds ?? []).find((b) => String(b[which] ?? "").toLowerCase().replace(/^0x/, "") === h) ?? null;
}

// ---- the words for a proposal -----------------------------------------------------------------------------------------------
const when = (ns) => new Date(Number(big(ns) / 1_000_000n)).toISOString().replace(/\.\d+Z$/, "Z");
const hours = (ns) => (Number(big(ns) / 1_000_000_000n) / 3600).toFixed(1);
const SEV_OF = { CRITICAL: "SEV1", WARN: "SEV2", INFO: "INFO" };
const worst = (sevs) => (sevs.includes("CRITICAL") ? "CRITICAL" : sevs.includes("WARN") ? "WARN" : "INFO");

/**
 * The alert for one proposal (a `ProposalView`, plain). `ctx`: { timelock, builds, baseline: { init, source } | null,
 * decoded: decodeInit(arg_hex) (for upgrades), roles: status.roles, controllers: the buyer's certified controllers }.
 */
export function proposalAlert(v, ctx) {
  const out = [], sevs = [];
  const line = (sev, text) => { sevs.push(sev); out.push(text); };
  out.push(`can run from ${when(v.executable_ns)} (${hours(big(v.executable_ns) - big(v.proposed_ns))} h after it was proposed)`);

  if (v.kind === "upgrade") {
    const hash = opt(v.wasm_hash), b = buildOf(ctx.builds, "buyer", hash);
    if (b) line("INFO", `module ${hash} (${opt(v.wasm_bytes)} bytes): the published reproducible build of ${b.commit}`);
    else line("CRITICAL", `module ${hash} (${opt(v.wasm_bytes)} bytes): UNKNOWN — not a published reproducible build. Veto unless a reproducible build of an announced commit prints this hash`);
    out.push(`argument sha256 ${opt(v.arg_hash)}`);
    const d = ctx.decoded;
    if (!d?.ok) line("CRITICAL", `argument: ${d?.error ?? "not decoded"}. Nothing below it can be trusted: veto unless explained`);
    if (d?.init && !ctx.baseline) line("CRITICAL", "no baseline for the running configuration (no executed upgrade in the history and no install argument configured): the argument cannot be diffed");
    if (d?.init && ctx.baseline) {
      const { lines, unchanged } = diffInit(ctx.baseline.init, d.init);
      out.push(`diff against ${ctx.baseline.source}:`);
      if (!lines.length) line("INFO", "  no configuration change (a code-only upgrade)");
      // review round 5 (the operator's option (c)): a diff that is exactly a staged step of the launch plan (ctx.staged: its number,
      // from staged.mjs stagedStep) is EXPECTED: its loosening is a WARN to check against the announcement, not a SEV1. The module
      // hash is judged as ever (an unknown build stays CRITICAL)
      const staged = Number.isInteger(ctx.staged) ? ctx.staged : null;
      if (staged) out.push(`  EXPECTED: this is step ${staged} of the launch plan (deployments/launch/nft-pilot.json stages; golive-check --proposal --stage ${staged} holds it): ${staged === 2 ? "the outflow raise" : "the item caps back to J7"}`);
      // review round 6 (A6-L-1, A6-I-4): only the lines the step itself names are EXPECTED (step 2: global_items; step 3: the item
      // counts of the caps it moves and the global item cap, their sum), and never one the running code refuses (×4); every other
      // LOOSENS line keeps its own severity (stagedStep already refuses a step that carries any other cap move)
      const own = (l) => {
        if (!staged || l.verb !== "LOOSENS" || !l.tag || /more than ×4/.test(l.text)) return false;
        if (Array.isArray(ctx.stagedTags)) return ctx.stagedTags.includes(l.tag);
        return staged === 2 ? l.tag === "limit:global_items" : staged === 3 && (/^cap:.*:max_items$/.test(l.tag) || l.tag === "global:global_max_items");
      };
      for (const l of lines) line(own(l) ? "WARN" : l.sev, `  ${l.verb} ${l.text}${own(l) ? " (expected: the staged step)" : ""}`);
      if (unchanged.length) out.push(`  unchanged: ${unchanged.join(", ")}`);
    }
    // review round 5 (A5-I-4): a proposal whose caps and limits do not fit the exit rule is refused by the running code at execute time
    if (d?.init) {
      const ex = exitProblemsOfInit(d.init);
      if (ex.length) line("CRITICAL", `  the argument does not fit the exit rule (every item must leave within the timelock's 72 h less a day): ${ex.join("; ")}: the running code refuses it, so executing it fails`);
    }
  } else if (v.kind === "controllers") {
    line("CRITICAL", `NEW CONTROLLERS for the buyer: ${list(opt(v.controllers))} (now, certified: ${list(ctx.controllers)}). This moves the buyer out from under this timelock`);
  } else if (v.kind === "roles") {
    const r = opt(v.roles), c = ctx.roles;
    out.push(`NEW ROLES: proposers ${list(r.proposers)}; guardians ${list(r.guardians)}; veto ${r.veto_threshold} of ${r.guardians.length}`);
    if (c) {
      out.push(`now: proposers ${list(c.proposers)}; guardians ${list(c.guardians)}; veto ${c.veto_threshold} of ${c.guardians.length}`);
      const gone = c.guardians.filter((g) => !r.guardians.includes(g)), newP = r.proposers.filter((p) => !c.proposers.includes(p));
      if (gone.length) line("CRITICAL", `LOOSENS removes or replaces guardian(s) ${gone.join(", ")}`);
      if (Number(r.veto_threshold) < Number(c.veto_threshold)) line("CRITICAL", `LOOSENS the veto threshold ${c.veto_threshold} → ${r.veto_threshold}`);
      if (newP.length) line("CRITICAL", `LOOSENS adds proposer(s) ${newP.join(", ")}`);
    }
    line("WARN", "a roles change replaces who may propose and who may veto");
  } else {
    line("CRITICAL", `a proposal kind this monitor does not know: ${v.kind}`);
  }

  const w = worst(sevs), sev = SEV_OF[w];
  const head = `PROPOSAL ${v.id} (${v.kind})${v.by ? ` by ${v.by}` : ""} on the buyer timelock ${ctx.timelock ?? ""}`.trim() +
    (w === "CRITICAL" ? " — CRITICAL" : "");
  const tail = w === "CRITICAL"
    ? "Guardians: veto unless this exact hash and diff were announced (H6); pause first if anything here is unexplained (G2 standing instruction 1)."
    : "Check it against its announcement (H6).";
  return { kind: "proposal", key: `proposal:${ctx.timelock}:${v.id}`, sev, text: [head, ...out, tail].join("\n") };
}
