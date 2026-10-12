// The keeper's and the resolver's liveness, read from 4663's own state every poll (docs/audit/2026-09-26/fixes-W2-monitor.md §1.2,
// fixes-B-C-keeper-draws.md §5 item 9, audit E-08): the pause, the stale fallback, a stale floor or cap, tickets and draws nobody
// resolves. Read on RPC A; a condition that would page is read again on RPC B, and paged only when B agrees.
import { DRAW_STAGE, MODULE, REGISTRY, TICKET_STAGE, VAULT, WRAPPER, host, read, roundTime } from "./abi.mjs";
import { stackOf } from "./stack.mjs";

export const STALE_FLOOR_MS = 15 * 60 * 1000;
export const NO_GOOD_POST_MS = 45 * 60 * 1000;
export const STALE_CAP_MS = 15 * 60 * 1000;
export const LATE_MS = 60 * 60 * 1000;           // a ticket or draw this long past its round (or announced this long ago)
export const SCAN_LIMIT = 200;
const ago = (ms) => (ms >= 3_600_000 ? `${(ms / 3_600_000).toFixed(1)} h` : `${Math.round(ms / 60_000)} min`);
const iso = (s) => new Date(Number(s) * 1000).toISOString();

/**
 * One poll. `readAt(url)`: a reader bound to one RPC ({ read(abi, to, fn, args) }). Returns { alerts, next }. `prev`: { since:
 * { key: ms }, drawFrom: { vault: id } }.
 */
export async function checkChain({ cfg, prev = {}, now = Date.now(), http = fetch }) {
  const st = stackOf(cfg), [a, b] = cfg.rpcs;
  const r = (u) => (abi, to, fn, args = []) => read(u, abi, to, fn, args, { http });
  const A = r(a), B = r(b);
  const alerts = [], since = {}, drawFrom = { ...(prev.drawFrom ?? {}) };
  /** A condition true now: remember since when (kept across polls), and page when it has held `forMs`, confirmed on B. */
  const held = async (key, isTrue, forMs, confirm, alert) => {
    if (!isTrue) return;
    since[key] = prev.since?.[key] ?? now;
    if (now - since[key] < forMs) return;
    const onB = await confirm().catch((err) => ({ error: String(err.message) }));
    if (onB?.error) { alerts.push({ kind: "provider-disagreement", sev: "SEV2", key: `chain-unconfirmed:${key}`, text: `${host(b)} could not confirm "${key}" (${onB.error.slice(0, 160)}); not paging it as fact.` }); return; }
    if (onB !== true) { alerts.push({ kind: "provider-disagreement", sev: "SEV2", persistent: true, key: `chain-disagree:${key}`, text: `the 4663 RPCs disagree about "${key}": ${host(a)} says it holds, ${host(b)} does not. Not guessing.` }); return; }
    alerts.push({ ...alert(now - since[key]), persistent: true, key: `${key}:${since[key]}` });
  };

  if (st.registry && cfg.guardian?.address) {
    // the guardian this monitor holds must be the one the registry names, or it cannot act when it must
    const named = String(await A(REGISTRY, st.registry, "guardian")).toLowerCase();
    await held("guardian-mismatch", named !== cfg.guardian.address.toLowerCase(), 0, async () => String(await B(REGISTRY, st.registry, "guardian")).toLowerCase() !== cfg.guardian.address.toLowerCase(),
      () => ({ kind: "guardian", sev: "SEV2", text: `the registry's guardian is ${named}, not this monitor's configured guardian ${cfg.guardian.address}: the monitor CANNOT pause the keeper or cancel a draw. The Safe's setGuardian(${cfg.guardian.address}), or remove evm.guardian from the config.` }));
  }
  if (st.registry) {
    const paused = await A(REGISTRY, st.registry, "keeperPaused");
    // r3-contracts N-1b, r4-contracts A3-M1: while paused every module still burns: at the frozen floor the keeper left, at most the
    // backing per unit (pauseFloor, R4), or at the backing per unit (exitAtBacking, an R3 module or a week with no good post)
    const eth = (w) => `${(Number(BigInt(w) / 10n ** 12n) / 1e6).toString()} ETH`;
    const exits = paused ? await Promise.all(st.collections.filter((c) => c.module).map(async (c) => {
      // r4-contracts: exitAtBacking() is escaped() alone (false while paused); burnsOpen() says a burn is priced at all
      const [pf, open, esc] = await Promise.all(["pauseFloor", "burnsOpen", "exitAtBacking"].map((f) => A(MODULE, c.module, f).catch(() => null)));
      if (esc === true) return `${c.slug} burns at the backing per unit (a week with no good post: exitAtBacking)`;
      if (pf !== null && BigInt(pf) > 0n) return `${c.slug} burns at the frozen floor ${eth(pf)} or its backing per unit, whichever is lower (pauseFloor)`;
      if (open === false) return `${c.slug} DOES NOT BURN (burnsOpen false while paused: check the module)`;
      return `${c.slug}: its paused burn price could not be read (pauseFloor ${pf === null ? "unreadable" : "0"}, burnsOpen ${open ?? "unreadable"})`;
    })) : [];
    await held("keeper-paused", paused, 0, () => B(REGISTRY, st.registry, "keeperPaused"), (d) => ({ kind: "keeper", sev: "SEV1",
      text: `THE KEEPER IS PAUSED (for ${ago(d)}): no floor, cap or draw is posted; mint, fills, sweeps and graduations (closing buys) are refused; draws cannot be pinned. Burns continue, less the base spread, from the moment of the pause: at the floor the keeper left (at most the backing per unit; the backing itself only after a week with no good post)${exits.length ? `: ${exits.join("; ")}` : ""}. The Safe unpauses (unpauseKeeper) once the cause is found and, if the key was stolen, the rotation (proposeRoles → 7 days → acceptRoles) has landed.` }));
  }
  for (const c of st.collections) {
    if (c.module) {
      const [fresh, escaped, postedAt, lastGoodAt] = await Promise.all([A(MODULE, c.module, "fresh"), A(MODULE, c.module, "escaped"), A(MODULE, c.module, "postedAt"), A(MODULE, c.module, "lastGoodAt")]);
      await held(`escaped:${c.module}`, escaped, 0, () => B(MODULE, c.module, "escaped"), () => ({ kind: "keeper", sev: "SEV1",
        text: `the ${c.slug} FloorModule is ESCAPED: no good floor post since ${iso(lastGoodAt)} (7 days + 1 h). Burns pay the backing per unit with no floor, and skim sends the surplus to the treasury. The keeper has been down, paused or posting confidence 0 for a week.` }));
      await held(`floor-stale:${c.module}`, !fresh, STALE_FLOOR_MS, async () => !(await B(MODULE, c.module, "fresh")), (d) => ({ kind: "keeper", sev: "SEV2",
        text: `the ${c.slug} floor is STALE (not fresh for ${ago(d)}; last post ${postedAt ? iso(postedAt) : "never"}): mint, fills and sweeps are refused and graduations wait. Check the keeper's /status (and UNISAT_API_KEY: without it no floor is agreed).` }));
      const goodAge = lastGoodAt ? now - Number(lastGoodAt) * 1000 : Infinity;
      await held(`no-good-post:${c.module}`, goodAge > NO_GOOD_POST_MS, 0, async () => { const g = await B(MODULE, c.module, "lastGoodAt"); return !g || now - Number(g) * 1000 > NO_GOOD_POST_MS; },
        () => ({ kind: "keeper", sev: "SEV2", text: `no good ${c.slug} floor post for ${lastGoodAt ? ago(goodAge) : "ever"} (confidence at or above the module's minimum): the keeper posts every 30 min when it can.` }));
      if (fresh) for (const [label, v] of [["RAFFLE", c.raffle], ["BURN", c.burn]]) {
        if (!v) continue;
        const capFresh = await A(VAULT, v, "capFresh");
        await held(`cap-stale:${v}`, !capFresh, STALE_CAP_MS, async () => !(await B(VAULT, v, "capFresh")), (d) => ({ kind: "keeper", sev: "SEV2",
          text: `the ${c.slug} ${label} vault's cap is STALE (for ${ago(d)}) while the floor is fresh: sweeps are refused. The keeper posts a cap about every 45 min; check its /status (a cap ceiling of 0 refuses every post).` }));
      }
    }
    if (c.wrapper) {
      const [next, count] = (await Promise.all([A(WRAPPER, c.wrapper, "nextToResolve"), A(WRAPPER, c.wrapper, "ticketCount")])).map(Number);
      for (let i = next; i < Math.min(count, next + SCAN_LIMIT); i++) {
        const t = await A(WRAPPER, c.wrapper, "ticket", [BigInt(i)]);
        if (TICKET_STAGE[Number(t.stage)] !== "Pinned") continue;
        const late = now / 1000 - roundTime(t.round);
        await held(`ticket-late:${c.wrapper}:${i}`, late * 1000 > LATE_MS, 0, async () => TICKET_STAGE[Number((await B(WRAPPER, c.wrapper, "ticket", [BigInt(i)])).stage)] === "Pinned",
          () => ({ kind: "keeper", sev: "SEV2", text: `${c.slug} redemption ticket ${i} is still PINNED ${ago(late * 1000)} after its drand round ${t.round} existed: nobody resolved it (the keyless resolver is down). Anyone can: Wrapper.resolve(${i}, <round ${t.round}'s signature>). It refunds 7 days after its pin (${iso(t.pinnedAt)}).` }));
      }
    }
    for (const [label, v] of [["RAFFLE", c.raffle], ["BURN", c.burn], ["YOYO", c.yoyo]]) {
      if (!v) continue;
      const count = Number(await A(VAULT, v, "drawCount"));
      let first = Math.min(drawFrom[v] ?? 0, count);
      let lowest = null;
      for (let i = first; i < Math.min(count, first + SCAN_LIMIT); i++) {
        const d = await A(VAULT, v, "draw", [BigInt(i)]);
        const stage = DRAW_STAGE[Number(d.stage)];
        if (["Announced", "Open", "Pinned", "Resolved"].includes(stage) && lowest === null) lowest = i;
        if (stage === "Announced") {
          const age = now - Number(d.announcedAt) * 1000;
          await held(`draw-announced-late:${v}:${i}`, age > LATE_MS, 0, async () => DRAW_STAGE[Number((await B(VAULT, v, "draw", [BigInt(i)])).stage)] === "Announced",
            () => ({ kind: "draw", sev: "SEV2", text: `${c.slug} ${label} draw ${i} was ANNOUNCED ${ago(age)} ago and is not open: the keeper did not open it (anyone may void it after 6 h: voidDraw(${i}); the void carries its seed round and snapshot time to the coin's next draw, review2 N-2).` }));
        } else if (stage === "Pinned") {
          const late = now / 1000 - roundTime(d.round);
          await held(`draw-pinned-late:${v}:${i}`, late * 1000 > LATE_MS, 0, async () => DRAW_STAGE[Number((await B(VAULT, v, "draw", [BigInt(i)])).stage)] === "Pinned",
            () => ({ kind: "draw", sev: "SEV2", text: `${c.slug} ${label} draw ${i} is still PINNED ${ago(late * 1000)} after its drand round ${d.round} existed: nobody resolved it (the resolver is down). Anyone can: resolve(${i}, <signature>). It voids 7 days after the pin.` }));
        } else if (stage === "Resolved") {
          // R2-S1: a resolved draw nobody claims (a forged file, a resolver that cannot prove the entry) used to be silent
          const age = now - Number(d.resolvedAt) * 1000;
          await held(`draw-resolved-unclaimed:${v}:${i}`, age > LATE_MS, 0, async () => DRAW_STAGE[Number((await B(VAULT, v, "draw", [BigInt(i)])).stage)] === "Resolved",
            () => ({ kind: "draw", sev: "SEV2", text: `${c.slug} ${label} draw ${i} was RESOLVED ${ago(age)} ago (winning ticket ${d.winningTicket}) and nobody has claimed it: the resolver cannot prove the winner's entry (a forged or missing file?). Anyone can claim with a proof from the chain (tools/snapshot --vault ${v} --draw-id ${i}); the site's Claim button rebuilds it. After 7 days anyone can void it and the prize returns to the coin.` }));
        }
      }
      drawFrom[v] = lowest ?? Math.min(count, first + SCAN_LIMIT);
    }
  }
  return { alerts, next: { since, drawFrom } };
}
