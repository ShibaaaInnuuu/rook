import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { decodeFunctionResult, encodeAbiParameters, encodeFunctionData, keccak256, parseAbi, toHex, type Abi, type Hex } from "viem";
import { stagingSender, STAGING_CHAIN_ID, type StagingSender } from "./guard.js";
import { BEACON, DISTRIBUTOR, REGISTRY, VAULT } from "../local/abis.js";
import { STAGING_MEMBERS, leafOf, memberId, membersForListing, proofForListing, proofOf } from "./members.js";
import { plan, type Action } from "../src/plan.js";
import { readListings, type ListingState } from "../src/chain.js";
import { FixtureQuoter, FixtureSource } from "../src/sources.js";
import { balancesOf, serialise, snapshot, type Snapshot } from "../../tools/snapshot/src/core.js";
import { transferLogs } from "../../tools/snapshot/src/rpc.js";

/**
 * docs/phase6.md, tier 1. The operator of our STAGING fork (its own chain id, staging/chain-id.json): every role a real deployment splits between a keeper, a
 * purchaser and an attestor, played by one process beside the node, sending AS the roles' random, keyless addresses through the
 * node's own impersonation (keeper/staging/guard.ts). Each pass reads the chain and does the next thing that is due, so a restart
 * loses nothing.
 *
 * REAL here: the contracts, the policy (src/plan.ts), the snapshot tool's roots over the fork's Transfer logs, drand rounds that did
 * not exist when they were pinned and their verification on chain.
 * FIXTURES, each marked `staging-fixture` where it is recorded: prices, the ETH→BTC quote, every Bitcoin receipt and delivery.
 * SHORTENED: the contracts' public waits are constants (an hour, fifteen minutes, an hour). Once a wait has been public for
 * STAGING_WAIT_S the operator rewrites that one stored deadline with the node's local admin method, because advancing the clock
 * would pin drand rounds in the real future. A real chain has no such method.
 */
const env = process.env;
const RPC = env.STAGING_NODE ?? "http://127.0.0.1:8546";
const DATA = env.STAGING_DATA ?? "/data";
const WAIT_S = Number(env.STAGING_WAIT_S ?? 120);
const CHOOSE_AFTER_S = Number(env.STAGING_CHOOSE_AFTER_S ?? 300);
const SETTLE_AFTER_S = Number(env.STAGING_SETTLE_AFTER_S ?? 45);
const MANUAL_SETTLE = env.STAGING_MANUAL_SETTLE === "1";
const TICK_MS = Number(env.STAGING_TICK_S ?? 10) * 1000;
const PUBLIC_BASE = (env.STAGING_PUBLIC_URL ?? "").replace(/\/$/, "");
const DRAND = "https://api.drand.sh/04f1e9062b8a81f848fded9c12306733282b2727ecced50032187751166ec8c3";
const HELD = parseAbi(["function inscriptionHeld(bytes32 leaf) view returns (bool)"]);
const ONCE = process.argv.includes("--once");
/** docs/phase11.md §7: how many otherwise-quiet listings a pass looks at, so 58 vaults do not cost 58 vaults' reads every tick. */
const WINDOW = Number(env.STAGING_LISTING_WINDOW ?? 12);
/** re-post a cap this long before it expires, so a vault never shows an expired price: keeper/src/plan.ts POLICY.repostIfUnderS. */
const CAP_MARGIN_S = 15 * 60;
const MIN_DECLARATION = 5n * 10n ** 16n;
const COUNT = parseAbi(["function count() view returns (uint256)"]);
const finishedLots = new Set<string>();
/** lots a creator chose to HOLD: nothing is due on them, but a creator may choose again, so they are re-read on the window's turn */
const parkedLots = new Set<string>();

const EXTRA = parseAbi([
  "function settleRune(uint256 id, uint256 amountReceived, bytes32 btcTxid, uint64 satsPaid)",
  "function openPurchases() view returns (uint256[])", "function isRune() view returns (bool)",
  "function requiredBondOf(address purchaser) view returns (uint256)", "function bondOf(address purchaser) view returns (uint256)",
  "function outstandingOf(address purchaser) view returns (uint256)",
  "function openDistribution(uint256 listingId, uint256 acquisitionId, bytes32 root, string uri)",
  "function distribution(uint256 listingId, uint256 acquisitionId) view returns ((bool open, bool rolled, uint64 challengeEnds, uint64 claimsEnd, bytes32 root, uint256 total, uint256 claimed, string uri))",
  "function carry(uint256 listingId, address coin) view returns (uint256)",
  "function payoutCount() view returns (uint256)", "function recordPayout(uint256 payoutId, bytes32 txid)",
  "function payout(uint256 payoutId) view returns ((address holder, bool overdue, uint64 requestedAt, uint256 listingId, uint256 amount, bytes32 txid, string destination))",
  "function creatorOf(address coin) view returns (address)", "function curveOf(address coin) view returns (address)",
  "function bondBps() view returns (uint16)",
  "function launcher() view returns (address)",
]);
const FACTORY = parseAbi(["function getLaunchedToken(address token) view returns (address token_, address curve, address deployer, address creatorFeeRecipient, address pairToken, uint256 graduationThreshold, uint24 poolFee, int24 tickSpacing, uint16 creatorTaxBps, bool buybackEnabled, uint8 phase, uint256 sweptQuote, uint256 sweptTokens, uint256 sweptAt, bool exists)"]);

type Dep = Record<string, string> & { fromBlock: number };
const log = (event: string, o: Record<string, unknown> = {}) => console.log(JSON.stringify({ at: new Date().toISOString(), svc: "staging-operator", event, ...o }, (_k, v) => (typeof v === "bigint" ? String(v) : v)));
const fixture = (what: string): Hex => keccak256(toHex(`staging-fixture: ${what}`));
const seen = new Map<string, number>();                       // when the operator first saw a thing waiting, by key
const noCapLogged = new Map<number, number>();                // a listing with no floor says so once an hour, not every tick
const cursors = new Map<number, number>();                    // where the last unheld member of a listing was found
const firstSeen = (key: string, nowS: number) => { if (!seen.has(key)) seen.set(key, nowS); return seen.get(key)!; };

/**
 * docs/phase8.md §4. What the simulator asks the operator to LEAVE ALONE, so that no state a page can show is ever empty: one
 * declaration still counting down, one release still awaiting its receipt, one challenge window still open, one delivery not yet
 * recorded. Without the simulator the file does not exist and nothing is held back.
 */
type Holds = { declarations: string[]; releases: string[]; challenges: string[]; deliveries: string[] };
const NO_HOLDS: Holds = { declarations: [], releases: [], challenges: [], deliveries: [] };
const holds = (): Holds => { try { return { ...NO_HOLDS, ...(JSON.parse(readFileSync(`${DATA}/sim-holds.json`, "utf8")) as Partial<Holds>) }; } catch { return NO_HOLDS; } };

export async function main(): Promise<void> {
  const s: StagingSender = await stagingSender(RPC);          // refuses unless KEEPER_MODE=staging, this machine, and the staging chain id
  const d = JSON.parse(readFileSync(`${DATA}/deployment.json`, "utf8")) as Dep;
  mkdirSync(`${DATA}/snapshots`, { recursive: true });
  const prices = `${DATA}/prices.json`;
  if (!existsSync(prices)) writeFileSync(prices, JSON.stringify({ inscriptions: { "demo-stones": "150000" }, runes: { "840000:1": "12" } }, null, 2) + "\n");
  const source: import("../src/sources.js").PriceSource = new FixtureSource(prices), quoter = new FixtureQuoter(3_200_000n, 1_500n, 60n);

  const read = async <T>(to: string, abi: Abi, functionName: string, args: unknown[] = []): Promise<T> => {
    const data = encodeFunctionData({ abi, functionName, args });
    return decodeFunctionResult({ abi, functionName, data: await s.request<Hex>("eth_call", [{ to, data }, "latest"]) }) as T;
  };
  const write = (from: string, to: string, abi: Abi, functionName: string, args: unknown[] = [], value = 0n) =>
    s.send({ from, to, data: encodeFunctionData({ abi, functionName, args }), value }).catch((err: Error) => { throw new Error(`${functionName} from ${from}: ${err.message}`); });
  const blockTime = async () => Number(BigInt((await s.request<{ timestamp: Hex }>("eth_getBlockByNumber", ["latest", false])).timestamp));
  /**
   * docs/phase11.md §8. The fork carries Robinhood Chain's block gas limit, 2^50, and anvil checks a sender's balance against
   * gas_limit x gas_price whenever it cannot estimate — which is exactly when the call would revert. At one gwei that asks for
   * about 1,125,900 ETH, so on 100 staging ETH every reverting call came back as "Insufficient funds for gas * price + value"
   * instead of its own error, and a real refusal was indistinguishable from an empty account. The roles are therefore kept far
   * above that line. The number is meaningless: this chain's ETH is its own, and nobody holds a key to any of these addresses.
   */
  const ROLE_FLOOR = 2_000_000n * 10n ** 18n;
  const ensureFunded = async (who: string, extra = 0n) => {
    const have = BigInt(await s.request<Hex>("eth_getBalance", [who, "latest"]));
    if (have >= ROLE_FLOOR + extra) return;
    await s.request("anvil_setBalance", [who, toHex(ROLE_FLOOR + extra)]);
  };
  /** Rewrites one stored uint64 to now (or `backBy` seconds ago). Only our own node has this method, and only from this machine. */
  const shorten = async (contract: string, slot: bigint, bitOffset: bigint, what: string, backBy = 0) => {
    const key = toHex(slot, { size: 32 });
    const word = BigInt(await s.request<Hex>("eth_getStorageAt", [contract, key, "latest"]));
    const mask = ((1n << 64n) - 1n) << bitOffset;
    await s.request("anvil_setStorageAt", [contract, key, toHex((word & ~mask) | (BigInt((await blockTime()) - backBy) << bitOffset), { size: 32 })]);
    await s.request("evm_mine");
    log("shortened-wait", { what, note: `staging only: it had been public for ${WAIT_S} s; a real chain waits it out` });
  };
  const purchaseSlot = (id: bigint) => BigInt(keccak256(toHex(17n, { size: 32 }))) + id * 7n;                  // AssetVault._purchases
  const nested = (base: bigint, listing: bigint, id: bigint) => BigInt(keccak256(encodeAbiParameters([{ type: "uint256" }, { type: "bytes32" }],
    [id, keccak256(encodeAbiParameters([{ type: "uint256" }, { type: "uint256" }], [listing, base]))])));

  /**
   * docs/phase11.md §7. Null when the coin has no holder yet — a coin that was launched and never bought, which a vault can
   * now buy for, because the simulator tops a vault up whether its coins have traded or not. The snapshot tool refuses to
   * root an empty set, rightly; this is not a failure, it is a lot that has to wait for someone to hold the coin.
   */
  async function snapshotFor(coin: string, kind: "weighted" | "per-wallet" | "pro-rata", vault: string, total?: bigint): Promise<{ snap: Snapshot; uri: string } | null> {
    const head = BigInt(await s.request<Hex>("eth_blockNumber"));
    const logs = await transferLogs(RPC, coin, BigInt(d.fromBlock), head);
    if (!logs.length) return null;
    const launcher = d.launcher;
    const token = await read<readonly unknown[]>(d.ponsFactory, FACTORY, "getLaunchedToken", [coin]);
    const listed = (JSON.parse(readFileSync(new URL("../../tools/snapshot/exclusions.json", import.meta.url), "utf8")) as { address: string }[]).map((x) => x.address);
    const snap = snapshot(balancesOf(logs), { coin, block: String(head), kind, decimals: 18, ...(total !== undefined ? { total: String(total) } : {}),
      exclusions: ["0x0000000000000000000000000000000000000000", "0x000000000000000000000000000000000000dead", coin, String(token[1]), String(token[3]), vault, launcher, ...listed] });
    const name = `snapshot-${coin.toLowerCase()}-${head}-${kind}.json`;
    writeFileSync(`${DATA}/snapshots/${name}`, serialise(snap));
    return { snap, uri: `${PUBLIC_BASE}/snapshots/${name}` };
  }
  const loadSnapshot = (uri: string): Snapshot | null => { try { return JSON.parse(readFileSync(`${DATA}/snapshots/${uri.split("/").pop()}`, "utf8")) as Snapshot; } catch { return null; } };

  /**
   * The purchaser's bond: the registry's share of everything in their hands, this purchase included. It is checked when a
   * purchase is RELEASED, and by then other purchases of the same vault may have been released too — so it is topped up both
   * before a declaration and again before each release, from what the vault itself reports rather than from a running total.
   */
  const ensureBond = async (l: ListingState, extraWei: bigint): Promise<void> => {
    const bps = BigInt(await read<number>(d.registry, EXTRA, "bondBps"));
    const outstanding = await read<bigint>(l.vault, EXTRA, "outstandingOf", [d.purchaser]);
    const need = ((outstanding + extraWei) * bps + 9_999n) / 10_000n;
    const have = await read<bigint>(l.vault, EXTRA, "bondOf", [d.purchaser]);
    if (have >= need) return;
    await ensureFunded(d.purchaser, need - have);
    await write(d.purchaser, l.vault, VAULT, "postBond", [], need - have);
    log("bond-posted", { listing: l.id, wei: need - have, covers: String(outstanding + extraWei), bondBps: String(bps) });
  };

  async function vaultPass(l: ListingState, nowS: number): Promise<void> {
    // 1. a cap and a declaration, by the published policy, from FIXTURE prices
    // docs/phase11.md §2. The catalogue carries no rune floor — a floor needs a marketplace, and every keyless one refuses an
    // unauthenticated read — so a real rune's vault is left with NO CAP, deliberately. What the contracts do then was checked,
    // not assumed: capExpiry is 0, so `declarePurchase` reverts CapExpired() and the vault simply refuses to buy.
    let price;
    try { price = l.kind === "inscriptions" ? await source.inscriptionFloor(l.ref) : await source.runeUnitPrice(l.ref, l.name); }
    catch (err) {
      const m = String((err as Error).message);
      if (!/has no (floor|price)/.test(m)) throw err;
      if (nowS - (noCapLogged.get(l.id) ?? -3600) >= 3600) {
        noCapLogged.set(l.id, nowS);
        log("no-cap-posted", { listing: l.id, kind: l.kind, ref: l.ref, why: `no floor exists for ${l.ref}: the catalogue carries none for a rune`,
          effect: "the vault refuses to declare a purchase (AssetVault.declarePurchase reverts CapExpired() while no cap has ever been posted)" });
      }
      return;
    }
    const size = l.spendableWei > 5n * 10n ** 16n ? l.spendableWei : 5n * 10n ** 16n;
    const actions: Action[] = plan(l, price, await quoter.quote(size), nowS, null);
    const cap = actions.find((a) => a.type === "postCap") as Extract<Action, { type: "postCap" }> | undefined;
    const decl = actions.find((a) => a.type === "declare") as Extract<Action, { type: "declare" }> | undefined;
    // docs/phase11.md §7. A cap lives an hour. It is re-posted whenever the policy says so — NOT only when a purchase is also
    // being declared, which left every vault that could not yet afford one showing an expired price until it could.
    if (cap) { await write(d.keeper, l.vault, VAULT, "postCap", [BigInt(cap.capWei)]); log("cap-posted", { listing: l.id, capWei: cap.capWei, price: "staging-fixture", why: cap.why }); }
    if (decl) {
      await ensureBond(l, BigInt(decl.maxEth));           // a release is refused without it, so it is posted before the declaration
      await write(d.keeper, l.vault, VAULT, "declarePurchase", [BigInt(decl.maxEth), BigInt(decl.quantity), `staging-fixture:${decl.capRef}`]);
      log("declared", { listing: l.id, maxEth: decl.maxEth, quantity: decl.quantity });
    }
    // 2. what is open: release once the wait has been public long enough; then a FIXTURE receipt
    for (const id of await read<bigint[]>(l.vault, EXTRA, "openPurchases")) {
      const p = await read<{ status: number; releasableAt: bigint; maxEth: bigint; quantity: bigint }>(l.vault, VAULT, "purchase", [id]);
      // a purchase id counts from zero in every vault, so a hold names the vault too
      const key = `${l.vault}:${id}`, holdKey = `${l.vault.toLowerCase()}:${id}`, held = holds();
      if (p.status === 1) {
        if (held.declarations.includes(holdKey)) continue;         // one declaration is left counting down, so a page always has one
        if (nowS < Number(p.releasableAt) && nowS - firstSeen(`declared:${key}`, nowS) >= WAIT_S) await shorten(l.vault, purchaseSlot(id), 8n, `the hour before purchase ${id} of listing ${l.id} may be released`);
        if ((await blockTime()) >= Number((await read<{ releasableAt: bigint }>(l.vault, VAULT, "purchase", [id])).releasableAt)) {
          // AssetVault.release checks the bond against everything in the purchaser's hands INCLUDING this release, and other
          // purchases of this vault have been released since this one was declared. So it is topped up here, not only there.
          await ensureBond(l, p.maxEth);
          await write(d.keeper, l.vault, VAULT, "release", [id]);
          log("released", { listing: l.id, purchase: id, to: d.purchaser, wei: p.maxEth });
        }
      } else if (p.status === 3 && held.releases.includes(holdKey)) {
        continue;                                                  // one release is left awaiting its receipt, for the same reason
      } else if (p.status === 3 && !MANUAL_SETTLE && nowS - firstSeen(`released:${key}`, nowS) >= SETTLE_AFTER_S) {
        const txid = fixture(`no Bitcoin transaction exists for purchase ${id} of listing ${l.id}; nothing was bought`);
        if (l.kind === "inscriptions") {
          // An inscription can be held once. For a catalogue listing the members are the few REAL ids the catalogue holds,
          // then the fixture ids that name nothing (docs/phase11.md §1): the real ones are used first, so what a visitor sees
          // delivered is a real inscription id for as long as we have one, and the fork never runs out after that.
          const m = membersForListing(DATA, l.id);
          const total = m ? m.ids.length : STAGING_MEMBERS;
          const idOf = (i: number) => (m ? m.ids[i] : memberId(i));
          let member = -1;
          for (let k = 0, from = cursors.get(l.id) ?? 0; k < total && member < 0; k++) {
            const i = (from + k) % total;
            if (!(await read<boolean>(l.vault, HELD, "inscriptionHeld", [leafOf(idOf(i))]))) member = i;
          }
          if (member < 0) { log("collection-exhausted", { listing: l.id, note: `all ${total} members of this listing's root are held: reset staging` }); continue; }
          cursors.set(l.id, member);
          await write(d.attestor, l.vault, VAULT, "settleInscription", [id, idOf(member), txid, 150_000n, m ? proofForListing(m, member) : proofOf(member)]);
          log("settled-inscription", { listing: l.id, purchase: id, inscription: idOf(member), real: m ? member < m.real : false,
            note: m && member < m.real ? "a real inscription id from the catalogue; the vault holds nothing, and nothing was bought" : "a staging fixture id that names nothing" });
        }
        else await write(d.attestor, l.vault, EXTRA, "settleRune", [id, p.quantity, txid, 1_000n]);
        log("settled", { listing: l.id, purchase: id, receipt: "staging-fixture", txid });
      }
    }
  }

  async function lotPass(l: ListingState, acq: bigint, nowS: number): Promise<void> {
    const L = BigInt(l.id), key = `${l.id}:${acq}`;
    type Lot = { mode: number; stage: number; challengeEnds: bigint; round: bigint; destinationAt: bigint; winner: string; winningTicket: bigint; uri: string; destination: string };
    const lot = await read<Lot>(d.distributor, DISTRIBUTOR, "lot", [L, acq]);
    // docs/phase11.md §7. Delivered, or chosen never to be distributed: nothing on chain moves it again, so it is never read
    // again. HELD is different — a creator may choose again out of Hold — so it is parked instead: read when its listing comes
    // round in the rotating window, not on every pass it is carried along by.
    if (lot.stage === 5 || lot.mode === 2) { finishedLots.add(key); parkedLots.delete(key); finishedByListing.set(l.id, (finishedByListing.get(l.id) ?? 0) + 1); return; }
    if (lot.mode === 1) { parkedLots.add(key); return; }
    parkedLots.delete(key);
    const a = await read<{ coin: string; amount: bigint }>(l.vault, VAULT, "acquisition", [acq]);
    if (lot.mode === 0) {       // nobody has chosen: after a while the operator chooses AS the creator, which only a staging node can do
      if (nowS - firstSeen(`unchosen:${key}`, nowS) < CHOOSE_AFTER_S) return;
      const creator = await read<string>(d.launcher, EXTRA, "creatorOf", [a.coin]);
      await ensureFunded(creator);
      await write(creator, d.distributor, DISTRIBUTOR, "choose", [L, acq, l.kind === "rune" ? 6 : 3, "0x0000000000000000000000000000000000000000"]);
      log("chosen-for-the-creator", { listing: l.id, acquisition: acq, mode: l.kind === "rune" ? "pro-rata" : "weighted draw", note: `staging only: unchosen for ${CHOOSE_AFTER_S} s` });
      return;
    }
    if (lot.mode === 6) {       // pro rata: publish who gets how much; holders claim for themselves
      const dist = await read<{ open: boolean; rolled: boolean; challengeEnds: bigint }>(d.distributor, EXTRA, "distribution", [L, acq]);
      if (!dist.open && !dist.rolled) {
        const total = a.amount + (await read<bigint>(d.distributor, EXTRA, "carry", [L, a.coin]));
        const taken = await snapshotFor(a.coin, "pro-rata", l.vault, total);
        if (!taken || taken.snap.holders === 0) return noHolders(l, acq, a.coin);
        const { snap, uri } = taken;
        await write(d.keeper, d.distributor, EXTRA, "openDistribution", [L, acq, snap.root, uri]);
        log("distribution-opened", { listing: l.id, acquisition: acq, holders: snap.holders, root: snap.root, uri });
      } else if (dist.open && nowS < Number(dist.challengeEnds) && nowS - firstSeen(`dist:${key}`, nowS) >= WAIT_S) {
        await shorten(d.distributor, nested(1n, L, acq), 16n, `the fifteen-minute challenge window of distribution ${key}`);
      }
      return;
    }
    // Mode.Named puts the lot straight into Stage.Claimed (src/Distributor.sol choose): its winner still posts an address and
    // its delivery is still recorded, so it takes the same stage-4 path a draw's winner takes. Stages 0-3 never happen for it.
    if (lot.mode !== 3 && lot.mode !== 4 && lot.mode !== 5) return;
    if (lot.stage === 0) {
      const taken = await snapshotFor(a.coin, lot.mode === 3 ? "weighted" : "per-wallet", l.vault);
      if (!taken || !taken.snap.holders) return noHolders(l, acq, a.coin);
      const { snap, uri } = taken;
      await write(d.keeper, d.distributor, DISTRIBUTOR, "openDraw", [L, acq, snap.root, BigInt(snap.totalTickets!), uri]);
      log("draw-opened", { listing: l.id, acquisition: acq, holders: snap.holders, tickets: snap.totalTickets, root: snap.root, uri });
    } else if (lot.stage === 1) {
      if (holds().challenges.includes(key)) return;                // one challenge window is left running, so the countdown is on screen
      if (nowS < Number(lot.challengeEnds) && nowS - firstSeen(`draw:${key}`, nowS) >= WAIT_S) await shorten(d.distributor, nested(0n, L, acq), 24n, `the fifteen-minute challenge window of draw ${key}`);
      if ((await blockTime()) >= Number((await read<Lot>(d.distributor, DISTRIBUTOR, "lot", [L, acq])).challengeEnds)) {
        await write(d.keeper, d.distributor, DISTRIBUTOR, "requestDraw", [L, acq]); log("draw-requested", { listing: l.id, acquisition: acq });
      }
    } else if (lot.stage === 2) {
      const due = Number(await read<bigint>(d.beacon, BEACON, "timeOf", [lot.round]));
      if (Date.now() / 1000 < due + 1) return;
      const r = await fetch(`${DRAND}/public/${lot.round}`, { headers: { "user-agent": "etched-staging/1" } }).catch(() => null);
      if (!r?.ok) return;
      const b = (await r.json()) as { round: number; signature: string };
      if (Number(b.round) !== Number(lot.round)) return;
      await write(d.keeper, d.distributor, DISTRIBUTOR, "resolve", [L, acq, `0x${b.signature}`]);
      log("draw-resolved", { listing: l.id, acquisition: acq, round: lot.round, beacon: "REAL: api.drand.sh, verified on chain" });
    } else if (lot.stage === 3) {
      const snap = loadSnapshot(lot.uri);
      const e = snap?.entries.find((x) => BigInt(x.first!) <= lot.winningTicket && lot.winningTicket < BigInt(x.end!));
      if (!e) return;
      await write(d.keeper, d.distributor, DISTRIBUTOR, "claimWin", [L, acq, e.holder, BigInt(e.first!), BigInt(e.end!), e.proof]);
      log("win-claimed", { listing: l.id, acquisition: acq, winner: e.holder });
    } else if (lot.stage === 4 && lot.destination) {
      if (holds().deliveries.includes(key)) return;                // one delivery is left unrecorded, so an overdue one is on screen
      const at = Number(lot.destinationAt);
      if (nowS < at + 3600 && nowS - firstSeen(`dest:${key}:${lot.destination}`, nowS) >= WAIT_S) await shorten(d.distributor, nested(0n, L, acq) + 1n, 0n, `the hour in which the winner of ${key} may correct their address`, 3601);
      if ((await blockTime()) >= Number((await read<Lot>(d.distributor, DISTRIBUTOR, "lot", [L, acq])).destinationAt) + 3600) {
        const txid = fixture(`no inscription was sent for ${key}; nothing exists to send`);
        await write(d.keeper, d.distributor, DISTRIBUTOR, "recordDelivery", [L, acq, txid]); log("delivery-recorded", { listing: l.id, acquisition: acq, delivery: "staging-fixture", txid });
      }
    }
  }

  /** Said once an hour per lot, not once a tick: a lot whose coin nobody holds yet is waiting, not failing. */
  const noHoldersLogged = new Map<string, number>();
  function noHolders(l: ListingState, acq: bigint, coin: string): void {
    const key = `${l.id}:${acq}`, nowS = Math.floor(Date.now() / 1000);
    if (nowS - (noHoldersLogged.get(key) ?? -3600) < 3600) return;
    noHoldersLogged.set(key, nowS);
    log("waiting-for-a-holder", { listing: l.id, acquisition: String(acq), coin, why: "nobody holds this coin yet, so there is nobody to draw between or pay pro rata" });
  }

  async function payoutPass(nowS: number): Promise<void> {
    const n = Number(await read<bigint>(d.distributor, EXTRA, "payoutCount"));
    for (let i = 0; i < n; i++) {
      const p = await read<{ txid: Hex }>(d.distributor, EXTRA, "payout", [BigInt(i)]);
      if (BigInt(p.txid) !== 0n || nowS - firstSeen(`payout:${i}`, nowS) < SETTLE_AFTER_S) continue;
      const txid = fixture(`no runes were sent for payout ${i}; nothing exists to send`);
      await write(d.keeper, d.distributor, EXTRA, "recordPayout", [BigInt(i), txid]); log("payout-recorded", { payout: i, delivery: "staging-fixture", txid });
    }
  }

  /**
   * docs/phase11.md §7. One pass used to read all seven state words of every listing and every lot of every vault, every tick.
   * With 58 vaults on a chain that is never reset, that grows with everything that has ever happened. It now reads:
   *
   *   - every listing that has WORK: an open purchase, a blocked vault, a cap inside its last quarter hour, enough to declare
   *     with, or a lot that is not finished — so nothing waiting is ever missed, however many vaults there are;
   *   - plus a rotating window of the rest, so a quiet vault is still looked at every minute or so;
   *   - and of their lots, only the ones that are not delivered.
   */
  const known = new Map<number, ListingState>();
  const lotCounts = new Map<number, number>(), finishedByListing = new Map<number, number>(), parkedByListing = new Map<number, number>();
  let cursor = 0;

  const pass = async () => {
    const nowS = Math.floor(Date.now() / 1000);
    for (const who of [d.keeper, d.purchaser, d.attestor]) await ensureFunded(who);
    const total = Number(await read<bigint>(d.registry, COUNT, "count"));
    const ids: number[] = [];
    for (let i = 0; i < total; i++) {
      const k = known.get(i);
      const unfinishedLots = (lotCounts.get(i) ?? 0) - (finishedByListing.get(i) ?? 0) - (parkedByListing.get(i) ?? 0);
      if (!k || k.open > 0 || k.blockedBy !== null || unfinishedLots > 0 || k.spendableWei >= MIN_DECLARATION
        || (k.capWei > 0n && k.capExpiry - nowS < CAP_MARGIN_S)) ids.push(i);
    }
    const inWindow = new Set<number>();
    for (let j = 0; j < WINDOW && ids.length + inWindow.size < total; j++) { const id = (cursor + j) % total; inWindow.add(id); if (!ids.includes(id)) ids.push(id); }
    cursor = total ? (cursor + WINDOW) % total : 0;
    let reads = 0;
    for (const l of await readListings(RPC, d.registry, fetch, ids)) {
      known.set(l.id, l);
      reads += 7;
      try { await vaultPass(l, nowS); } catch (err) { log("vault-pass-failed", { listing: l.id, error: String((err as Error).message).slice(0, 240) }); }
      const n = Number(await read<bigint>(l.vault, VAULT, "acquisitionCount"));
      lotCounts.set(l.id, n);
      reads++;
      let parked = 0;
      for (let i = 0; i < n; i++) {
        const key = `${l.id}:${i}`;
        if (finishedLots.has(key)) continue;
        if (parkedLots.has(key)) { parked++; if (!inWindow.has(l.id)) continue; }
        reads++;
        try { await lotPass(l, BigInt(i), nowS); } catch (err) { log("lot-pass-failed", { listing: l.id, acquisition: i, error: String((err as Error).message).slice(0, 240) }); }
      }
      parkedByListing.set(l.id, parked);
    }
    try { await payoutPass(nowS); } catch (err) { log("payout-pass-failed", { error: String((err as Error).message).slice(0, 240) }); }
    if (nowS - lastTallyS >= 60) { lastTallyS = nowS; log("pass", { listings: total, read: ids.length, listingReads: reads, note: "a pass reads the listings with work, plus a rotating window of the rest" }); }
  };
  let lastTallyS = 0;
  log("start", { rpc: RPC, waitS: WAIT_S, chooseAfterS: CHOOSE_AFTER_S, manualSettle: MANUAL_SETTLE, chainId: STAGING_CHAIN_ID, note: "the staging chain only; no key exists; receipts and prices are fixtures" });
  if (ONCE) { await pass(); return; }
  for (;;) { await pass(); await new Promise((ok) => setTimeout(ok, TICK_MS)); }
}

if (import.meta.url === `file://${process.argv[1]}`) main().catch((err) => { log("fatal", { error: String((err as Error).message) }); process.exit(1); });
