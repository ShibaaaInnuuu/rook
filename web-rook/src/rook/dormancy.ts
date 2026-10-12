/**
 * docs/v7/dormant-collections.md §4.2: what the Launch page says about a collection the canister keeps asleep. Every allowlisted
 * collection is deployed, but a DORMANT one has no floor posted (its factory refuses a launch that buys items until the canister's first
 * confirmed post), so the page offers the wake: a bonded request on the WakeDesk, the bond back when you launch. While it warms up the
 * page says "warming up (~N min)" (the canister's own estimate, from the runner's GET /collections); once ACTIVE it launches as before.
 * Pure: the page fetches the lives and the stack file's desk and bond, this decides what to show and whether to send.
 */

/** One collection's life, as the runner's GET /collections serves `eth_nft_collections()` (nat64 as strings, opt as arrays). */
export type Life = {
  collection: string; wrapper: string; module: string;
  state: "always-on" | "dormant" | "queued" | "waking" | "active" | string;
  since: string | number; eta_secs: (string | number)[]; queue_position: number[]; woken_by: string[]; cooldown_until: (string | number)[];
  status: string; note: string;
};

/** The lives from GET /collections, checked; null for anything that is not a list of them (the page then says nothing about dormancy). */
export function livesFrom(body: unknown): Life[] | null {
  if (!Array.isArray(body)) return null;
  const ok = body.every((x) => x && typeof x === "object" && typeof (x as Life).wrapper === "string" && typeof (x as Life).state === "string"
    && typeof (x as Life).status === "string");
  return ok ? (body as Life[]) : null;
}

/** The life of the collection whose wrapper is `wrapper`, or null (not served, or no dormancy). */
export function lifeOf(lives: Life[] | null, wrapper: string | null | undefined): Life | null {
  if (!lives || !wrapper) return null;
  return lives.find((l) => l.wrapper.toLowerCase() === wrapper.toLowerCase()) ?? null;
}

export type WakeView = {
  /** the summary's word and tone */
  word: string; tone: "good" | "warn" | "muted";
  /** why a launch that buys items waits (null: it does not) */
  launchWait: string | null;
  /** whether the page offers the bonded wake request */
  canWake: boolean;
  detail: string;
};

/** The collection's open desk request, as the page read it on chain (`openOf`, then `requestAt` for when it was asked). */
export type OpenRequest = { id: bigint | number | string; at?: number };

/** What the page knows besides the life: whether the stack names a desk (the dormancy), the module's on-chain freshness and when its
 *  floor was last posted (`postedAt`, seconds; 0: never), the desk's window and the time now (seconds). */
export type WakeCtx = { dormancy?: boolean; fresh?: boolean; postedAt?: number; windowSecs?: number; nowS?: number };

/** The canister reads new desk requests every tick; its idle cadence is at most an hour (ops RUNNER_IDLE_TICK_EVERY_S). */
export const READ_WITHIN_S = 3_600;

/**
 * Independent review 2 (D2-I-1): with no lives served, how long since the module's last post before the page takes "not fresh" to mean
 * dormant. An ACTIVE collection posts about every 40 minutes, well inside the module's FLOOR_TTL (3 h since 2026-10-09; the canister still plans a pass 25 minutes before the hour), so a module
 * briefly stale (a post pending past the TTL, a late hand-in) was posted under two hours ago: the page does not offer a bond that would
 * only pay for that routine post. A collection that slept has its last post from before it slept.
 */
export const NO_LIVES_OFFER_AFTER_S = 2 * 3_600;

/** The wording of the desk's one rule, where a request is said to go back: any post inside the window serves it (D2-L-3). */
export const SERVED_RULE = "unless the collection's floor is posted inside the window by another wake (a deposit, a graduation or the "
  + "operator's): the desk counts any post inside the window as serving the request";

/**
 * Independent review DI-M-2: what an open request means, said from the canister's life, not from `openOf` alone. A DORMANT life with an
 * open request either has not been read yet (asked under an hour ago) or could not be served inside the request's window (the canister
 * drops a request whose wake could no longer post inside it, and the desk takes a new request once that window ends). Independent
 * review 2 (D2-L-3): its bond goes back unless another wake posts the collection inside the window, which is what the desk does.
 */
export function openText(o: OpenRequest, ctx: WakeCtx = {}): string {
  const id = String(o.id);
  const ends = o.at !== undefined && ctx.windowSecs !== undefined ? o.at + ctx.windowSecs : undefined;
  const when = ends !== undefined ? ` (${new Date(ends * 1000).toISOString().replace("T", " ").slice(0, 16)} UTC)` : "";
  if (o.at !== undefined && ctx.nowS !== undefined && ctx.nowS - o.at < READ_WITHIN_S) {
    return `A wake is already asked for this collection (request #${id}); the canister reads it at its next tick and queues it.`;
  }
  return `A wake was asked for this collection (request #${id}), but the canister is not warming it up: it could not start that wake in time `
    + `to serve it inside the request's window. That bond goes back to its requester (WakeDesk.forfeit) ${SERVED_RULE}. A new request can be `
    + `made once the window ends${when}.`;
}

/**
 * What the page shows for a collection's life. `open`: the desk's open request for it, read on chain (`true` or the request; `false`:
 * none; `null`: not read yet, so no wake is offered). `ctx.fresh === true`: the module is fresh on chain, so the collection is awake
 * and the desk refuses a request (DI-M-2): none is offered. With no life served (no `VITE_COLLECTIONS_URL`, or the relay down) and the
 * dormancy on, a module read NOT fresh offers the wake, gated on the `openOf` read (DI-L-1).
 */
export function wakeView(l: Life | null, open: boolean | OpenRequest | null = false, ctx: WakeCtx = {}): WakeView {
  const isOpen = open === true || (typeof open === "object" && open !== null);
  const openMsg = typeof open === "object" && open !== null ? openText(open, ctx)
    : "A wake is already asked for this collection; the canister queues it at its next tick if it can still be served inside the request's window.";
  const offer = open === false && ctx.fresh !== true;
  if (!l || l.state === "always-on") {
    if (ctx.dormancy && ctx.fresh === false) {
      if (isOpen) return { word: "Wake asked", tone: "warn", canWake: false, launchWait: openMsg, detail: "" };
      // D2-I-1: without the canister's lives, "not fresh" means dormant only once the module has been stale longer than a post cadence
      const since = ctx.postedAt !== undefined && ctx.nowS !== undefined ? ctx.nowS - ctx.postedAt : undefined;
      if (since === undefined || (ctx.postedAt !== 0 && since < NO_LIVES_OFFER_AFTER_S)) {
        return { word: "Not fresh", tone: "muted", canWake: false,
          launchWait: since === undefined
            ? "This collection's floor is not fresh on chain. Reading when it was last posted before offering a wake."
            : "This collection's floor is not fresh on chain, but it was posted under two hours ago: the collection is probably awake and its next floor post is late. A coin that buys NFTs launches once it is posted again; no wake is offered, since its bond would only pay for that routine post.",
          detail: "" };
      }
      return { word: "Not fresh", tone: "muted", canWake: offer,
        launchWait: "This collection's floor is not fresh on chain and was last posted over two hours ago: it is probably dormant (nobody has launched on it lately). " + FIRST_COIN,
        detail: offer ? `${RECLAIM_NOTE} ${NO_LIVES_NOTE}` : "" };
    }
    return { word: "Awake", tone: "good", launchWait: null, canWake: false, detail: "" };
  }
  if (l.state === "active") return { word: "Awake", tone: "good", launchWait: null, canWake: false, detail: "" };
  if (l.state === "waking") {
    return { word: l.status || "warming up", tone: "warn", canWake: false,
      launchWait: `${l.status || "Warming up"}: the canister is reading this collection's last 7 days of sales to confirm its floor. You can launch now: ${GRADUATES_LATER}`,
      detail: "Nothing is bought and nothing buys until the floor is confirmed by real sales; then the first floor post goes out and launches open." };
  }
  if (l.state === "queued") {
    return { word: l.status || "queued", tone: "warn", canWake: false,
      launchWait: `${l.status || "Queued"}: a wake is already asked for; it starts when the canister has room (how many collections can be awake follows its running budget). You can launch now: ${GRADUATES_LATER}`,
      detail: "Wakes are bounded by what the canister's running budget covers, so its costs stay covered; yours is in line." };
  }
  return { word: "Dormant", tone: "muted", canWake: offer,
    launchWait: isOpen ? openMsg
      : "Dormant: nobody has launched on this collection lately, so its floor is not watched. " + FIRST_COIN,
    detail: [l.status, isOpen ? "" : RECLAIM_NOTE].filter(Boolean).join(" ") };
}

/** The operator, 2026-10-09: launching never waits for the floor. The contract holds a graduation until a fresh floor is posted
 *  (`FloorModule.enqueue` queues the item budget; `fill` converts it only while `fresh()`), and refunds open when the curve's window ends. */
export const GRADUATES_LATER = "your coin trades right away and graduates once the collection's floor is live (about 40 minutes after a wake starts). "
  + "If the floor is never posted, the coin cannot graduate and its buyers take their ETH back when its window ends.";

/** The first coin on a sleeping collection wakes it, as part of launching: step 1 the bonded wake, step 2 the launch. */
export const FIRST_COIN = "You are the first coin on it, so launching wakes it: step 1 asks for the wake (a bond, back when your coin "
  + "graduates), step 2 launches your coin. After that " + GRADUATES_LATER;

/** dormant-review DR-M-4: what the bond asks for, said where it is offered. */
export const RECLAIM_NOTE = "The bond comes back (WakeDesk.reclaim) once a coin you launch on this collection after asking has graduated, "
  + "within the desk's window. If the collection's floor is not posted inside that window (your wake was not served in time, "
  + "even if it is served later), anyone may return the bond to you (WakeDesk.forfeit refunds it; an address that does not take it is "
  + "credited, for WakeDesk.withdraw); otherwise an unreclaimed bond pays the treasury for the scan. Any post inside the window serves "
  + "the request, whatever woke the collection.";

/** D2-I-1: said where the wake is offered without the canister's lives (the page cannot see whether the collection is awake). */
export const NO_LIVES_NOTE = "The page cannot see the canister's own view of this collection right now. If it is in fact awake and its "
  + "floor post is only late, that next post serves your request and the bond pays for it.";

/**
 * What a launch waits for. The operator, 2026-10-09: never the floor (the contract holds a graduation until a fresh floor is posted, and
 * refunds when the window ends, so a coin launched on a stale or sleeping collection cannot graduate against a stale price). It waits only
 * for the wake request when the collection needs one and nobody has asked (`view.canWake`): that is step 1 of the first coin's launch.
 * A floor-buys-only coin never waits. Returns why the launch waits, or null. (dormant-review DR-M-4 made the page fail closed on the
 * chain's `fresh()`; the chain's own graduation gate is that check now, and the page still never trusts GET /collections to open a launch.)
 */
export function launchWait(o: { dormancy: boolean; sweepOnly: boolean; fresh: boolean | undefined; view: WakeView }): string | null {
  if (o.sweepOnly || !o.dormancy) return null;
  if (o.fresh === true) return null;
  return o.view.canWake ? "Step 1 first: ask for the wake below. Then launch your coin." : null;
}

/**
 * docs/v7/dormant-collections.md §11: how many collections the canister's own cycles balance keeps awake right now (the runner's
 * GET /capacity serves `eth_nft_capacity()`: nat as strings). The limit grows with top-ups up to a governance ceiling; a collection that
 * holds positions stays awake whatever the balance says.
 */
export type Capacity = { effective_max_active: number; collections: number; ceiling: number; awake: number; pinned: number; wake_headroom: number;
  wakes_left_today: number; scaling: boolean; status: string };

/** The capacity from GET /capacity, checked; null for anything else (the page then says nothing about it). */
export function capacityFrom(body: unknown): Capacity | null {
  if (!body || typeof body !== "object" || Array.isArray(body)) return null;
  const c = body as Record<string, unknown>;
  const nums = ["effective_max_active", "collections", "ceiling", "awake", "pinned", "wake_headroom", "wakes_left_today"];
  if (!nums.every((k) => typeof c[k] === "number" && Number.isInteger(c[k]) && (c[k] as number) >= 0 && (c[k] as number) <= 255)) return null;
  if (typeof c.scaling !== "boolean" || typeof c.status !== "string") return null;
  if ((c.effective_max_active as number) > (c.collections as number)) return null;
  return c as unknown as Capacity;
}

/** "N of M collections can be awake right now", and what it means for a wake asked now. */
export function capacityText(c: Capacity | null): { line: string; note: string } | null {
  if (!c) return null;
  const line = `${c.effective_max_active} of ${c.collections} collections can be awake right now`;
  const note = c.wake_headroom > 0
    ? `${c.awake} ${c.awake === 1 ? "is" : "are"} awake; a wake asked now can start.`
    : c.awake > c.effective_max_active || c.effective_max_active === 0
      ? `${c.awake} ${c.awake === 1 ? "is" : "are"} awake${c.pinned ? ` (${c.pinned} with holders, kept awake)` : ""}; a new wake waits in the queue until the canister's running budget is topped up or one goes back to sleep.`
      : `${c.awake} ${c.awake === 1 ? "is" : "are"} awake; a new wake waits in the queue for room.`;
  return { line, note: `${note} The number follows the canister's running budget (its cycles): it grows as the budget does.` };
}

/** Where GET /capacity is: next to the runner's GET /collections (VITE_COLLECTIONS_URL, absolute or a same-origin path), or null. */
export function capacityUrl(collectionsUrl: string | null | undefined): string | null {
  if (!collectionsUrl) return null;
  if (collectionsUrl.startsWith("/") && !collectionsUrl.startsWith("//")) {
    const path = collectionsUrl.split(/[?#]/)[0];
    return /\/collections\/?$/.test(path) ? path.replace(/\/collections\/?$/, "/capacity") : null;
  }
  try {
    const u = new URL(collectionsUrl);
    if (!/\/collections\/?$/.test(u.pathname)) return null;
    u.pathname = u.pathname.replace(/\/collections\/?$/, "/capacity");
    return u.toString();
  } catch { return null; }
}

/** The bond the stack file names (wei), or null when it names no desk or no bond: no wake is offered then. */
export function bondOf(stack: { wakeDesk?: string; wakeBond?: string | number } | null): bigint | null {
  if (!stack?.wakeDesk || !/^0x[0-9a-fA-F]{40}$/.test(stack.wakeDesk) || /^0x0{40}$/.test(stack.wakeDesk)) return null;
  try { return stack.wakeBond !== undefined ? BigInt(stack.wakeBond) : null; } catch { return null; }
}
