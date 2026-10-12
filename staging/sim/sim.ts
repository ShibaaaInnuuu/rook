import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { encodeFunctionData, keccak256, parseAbi, parseAbiItem, toHex, type Hex } from "viem";
import { openSim, type Sim } from "./wallets.js";
import { buy, harvest, launch, phaseOf, sell, type Launched } from "./trade.js";
import { burstEndsAt, coinName, gapMs, globalGapMs, logo, moodFor, nextLaunchDelayMs, nextStep, shouldBurst, shouldSnipe, taxFor, type Coin, type Mood } from "./moods.js";
import { choiceFor, spread, taprootFor, type Census, type LotCensus, type PurchaseCensus } from "./spread.js";
import { STAGING_CHAIN_ID } from "../chain-id.js";
import { launchOrder } from "../catalogue/listings.js";
import { readManifest } from "../bootstrap/listings.js";

/**
 * docs/phase8.md. The staging simulators: coins launching, curves being traded, vaults filling, and every state a page can show
 * held on screen at once. It runs beside the node, on this machine, and nowhere else.
 *
 * Nothing here is worth anything. Every wallet is made at boot and thrown away, every price and receipt is a fixture, and the
 * chain is our own copy (staging/chain-id.json). The operator (keeper/staging/operator.ts) does the keeper's, purchaser's and
 * attestor's work; this only makes a market for it and keeps the rare states occupied.
 */
const env = process.env;
const RPC = env.STAGING_NODE ?? "http://127.0.0.1:8546";
const DATA = env.STAGING_DATA ?? "/data";
const ON = (k: string) => (env[k] ?? "1") !== "0";
const MAX_COINS = Number(env.STAGING_SIM_COINS ?? 64);            // docs/phase11.md §7: one for every vault, and a few over
const WALLETS = Number(env.STAGING_SIM_WALLETS ?? 30);
const BASE_MS = Number(env.STAGING_SIM_EVERY_MS ?? 4_000);
const HARVEST_EVERY_MS = Number(env.STAGING_SIM_HARVEST_MS ?? 45_000);
const SPREAD_EVERY_MS = Number(env.STAGING_SIM_SPREAD_MS ?? 20_000);
/** how many vaults' counts are re-read per spread pass: the rest keep the counts they had, and their live items are read anyway */
const SPREAD_WINDOW = Number(env.STAGING_SIM_SPREAD_WINDOW ?? 12);
const TOPUP_EVERY_MS = Number(env.STAGING_SIM_TOPUP_MS ?? 12_000);
const LISTINGS_EVERY_MS = Number(env.STAGING_SIM_LISTINGS_MS ?? 30_000);
const MAX_CONSECUTIVE_FAILURES = 50;

const REGISTRY = parseAbi(["function count() view returns (uint256)",
  "function listing(uint256 id) view returns ((uint8 kind, bool paused, uint8 divisibility, address vault, bytes32 membershipRoot, string name, string ref, string btcVault))",
  "function owner() view returns (address)"]);
const VAULT = parseAbi(["function purchaseCount() view returns (uint256)", "function acquisitionCount() view returns (uint256)",
  "function purchase(uint256 id) view returns ((uint8 status, uint64 releasableAt, uint64 deadline, address purchaser, uint256 maxEth, uint256 quantity, uint256 settledQuantity, uint256 returned, string capRef))",
  "function returnUnspent(uint256 id) payable", "function writeOff(uint256 id, bytes32 reasonRef)",
  "function acquisition(uint256 id) view returns ((uint256 purchaseId, address coin, uint256 amount, bytes32 btcTxid, uint64 satsPaid, string assetRef))",
  "function capWei() view returns (uint256)", "function capExpiry() view returns (uint64)",
  "function attributed() view returns (uint256)", "function reserved() view returns (uint256)",
  "function depositFrom(address coin) payable"]);
const DIST = parseAbi(["function choose(uint256 listingId, uint256 acquisitionId, uint8 mode, address named)",
  "function setDestination(uint256 listingId, uint256 acquisitionId, string taprootAddress)",
  "function lot(uint256 listingId, uint256 acquisitionId) view returns ((uint8 mode, uint8 stage, bool overdue, uint64 challengeEnds, uint64 round, uint64 pinnedAt, uint64 destinationAt, address winner, bytes32 root, uint256 totalTickets, uint256 winningTicket, bytes32 deliveryTxid, string uri, string destination))"]);

type Dep = Record<string, string> & { fromBlock: number };
const log = (event: string, o: Record<string, unknown> = {}) =>
  console.log(JSON.stringify({ at: new Date().toISOString(), svc: "staging-sim", event, ...o }, (_k, v) => (typeof v === "bigint" ? String(v) : v)));
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const rand = () => Math.random();

/** keeper/src/plan.ts POLICY.minDeclarationWei: under this the keeper declares nothing, whatever the cap says. */
const MIN_DECLARATION = 5n * 10n ** 16n;

/** Our Launcher's own event, so a restarted simulator can find the coins this chain already has. */
const LAUNCHED_EVENT = parseAbiItem("event Launched(address indexed coin, address indexed creator, uint256 indexed listingId, address curve, address router)");

/** Why a purchase was written off. It names itself a fixture, as every made-up thing on staging does. */
const FIXTURE_REASON = keccak256(toHex("staging-fixture: no receipt ever came, because nothing was ever bought"));

/** The tally the once-a-minute line reports. */
const tally = { launches: 0, buys: 0, sells: 0, harvests: 0, graduations: 0, failures: 0, chosen: 0, destinations: 0, returned: 0, writtenOff: 0, toppedUp: 0 };

export async function main(): Promise<void> {
  if (!ON("STAGING_SIM")) { log("off", { why: "STAGING_SIM=0" }); return; }
  const s = await openSim(RPC, WALLETS);
  const d = JSON.parse(readFileSync(`${DATA}/deployment.json`, "utf8")) as Dep;
  mkdirSync(DATA, { recursive: true });
  log("start", { chainId: STAGING_CHAIN_ID, wallets: s.wallets.length, maxCoins: MAX_COINS,
    note: "every wallet, price and receipt here is made up; the keys exist only in this process and are never written" });

  const read = <T>(to: string, abi: readonly unknown[], functionName: string, args: unknown[] = []) =>
    s.pub.readContract({ address: to as Hex, abi: abi as never, functionName, args }) as Promise<T>;
  /**
   * Sends AS one of the deployment's roles. Nobody holds a key to any of them (staging/bootstrap: twenty random bytes each);
   * the node signs for whoever asks from this machine, and the public proxy refuses this method to everyone else.
   */
  const asRole = async (from: string, to: string, data: Hex, value = 0n) => {
    const hash = await s.admin<Hex>("eth_sendTransaction", [{ from, to, data, value: `0x${value.toString(16)}`, gas: "0x300000" }]);
    if ((await s.mined(hash)).status !== "success") throw new Error(`reverted: ${data.slice(0, 10)} to ${to}`);
  };
  /**
   * Moves one purchase's stored receipt deadline into the past. `AssetVault._purchases` is at slot 17 (docs/phase3.md pinned it
   * there); each Purchase takes seven slots, and its first packs status(8) | releasableAt(64) | deadline(64), so the deadline
   * begins at bit 72. Only our own node answers `anvil_setStorageAt`, and only from this machine.
   */
  const ageDeadline = async (vault: string, id: number) => {
    const slot = toHex(BigInt(keccak256(toHex(17n, { size: 32 }))) + BigInt(id) * 7n, { size: 32 });
    const word = BigInt(await s.admin<Hex>("eth_getStorageAt", [vault, slot, "latest"]));
    const mask = ((1n << 64n) - 1n) << 72n;
    const past = BigInt(Math.floor(Date.now() / 1000) - 3600);
    await s.admin("anvil_setStorageAt", [vault, slot, toHex((word & ~mask) | (past << 72n), { size: 32 })]);
    await s.admin("evm_mine");
  };

  /**
   * The listings the registry holds. docs/phase11.md §7: the catalogue's inventory is created BESIDE the proxy, over the first
   * minute or so, so this is re-read rather than taken once. The manifest the creator writes carries each listing's weight (the
   * catalogue's own all-time volume) and its kind, so a refresh costs one read of the registry's count, not 58 reads of it.
   */
  type SimListing = { id: number; vault: string; isRune: boolean; kind: "inscriptions" | "rune"; ref: string; weight: number };
  let listings: SimListing[] = [];
  const loadListings = async (): Promise<void> => {
    const n = Number(await read<bigint>(d.registry, REGISTRY, "count"));
    if (n === listings.length) return;
    const byId = new Map((readManifest(DATA)?.listings ?? []).map((l) => [l.id, l]));
    const out: SimListing[] = [];
    for (let i = 0; i < n; i++) {
      const known = byId.get(i);
      if (known) { out.push({ id: i, vault: known.vault, isRune: known.kind === "rune", kind: known.kind, ref: known.ref, weight: known.weight }); continue; }
      const l = await read<{ kind: number; vault: string; ref: string }>(d.registry, REGISTRY, "listing", [BigInt(i)]);
      out.push({ id: i, vault: l.vault, isRune: l.kind !== 0, kind: l.kind === 0 ? "inscriptions" : "rune", ref: l.ref, weight: 1 });
    }
    const was = listings.length;
    listings = out;
    order = launchOrder(listings, MAX_COINS);
    if (out.length !== was) log("listings-read", { listings: out.length, was, note: "the inventory fills beside the node; this is re-read as it does" });
  };
  /** Which listing each coin launches into: popular collections sooner and again, every fourth coin a rune. */
  let order: SimListing[] = [];
  await loadListings();
  if (!listings.length) { log("nothing-to-do", { why: "the registry holds no listing" }); return; }


  const coins: Coin[] = [];
  const creatorOf = new Map<string, number>();       // coin -> which wallet launched it, so it can choose as the creator

  /**
   * The coins this chain already has, from our Launcher's own `Launched` events. A restart makes new wallets and remembers
   * nothing, and once the launch order is finished it launches nothing either — so without this, a restarted simulator would
   * leave a chain of 58 vaults completely silent. Rehydrated coins are bought but not sold: the wallets that hold them are
   * gone, and no key to them was ever written. Their creator is gone too, so the operator's fallback chooses their lots.
   */
  const rehydrate = async (): Promise<void> => {
    const logs = await s.pub.getLogs({ address: d.launcher as Hex, event: LAUNCHED_EVENT, fromBlock: BigInt(d.fromBlock ?? 0), toBlock: "latest" });
    for (const lg of logs) {
      const a = (lg as unknown as { args: { coin: Hex; listingId: bigint; curve: Hex; router: Hex } }).args;
      if (!a?.coin || coins.some((c) => c.address.toLowerCase() === a.coin.toLowerCase())) continue;
      const { phase } = await phaseOf(s, d.ponsFactory, a.coin);
      coins.push({ address: a.coin, curve: a.curve, router: a.router, listingId: Number(a.listingId), mood: moodFor(coins.length),
        phase: phase >= 1 ? "graduated" : "curve", bornMs: Date.now() - 3_600_000, burstUntilMs: 0, bought: 0, sniped: true });
    }
    if (coins.length) log("coins-rehydrated", { coins: coins.length, note: "launched before this process started; they are bought, never sold, because the wallets that hold them are gone" });
  };
  let nextLaunchAt = Date.now(), nextTradeAt = Date.now() + 2_000, nextHarvestAt = Date.now() + 20_000, nextSpreadAt = Date.now() + 30_000;
  let nextReportAt = Date.now() + 60_000, nextListingsAt = Date.now() + LISTINGS_EVERY_MS, nextTopUpAt = Date.now() + 20_000;
  let failures = 0, chosenCount = 0, turn = 0;

  /**
   * Where the launch order has got to, kept on the volume. A restart makes new wallets and forgets the coins it launched, and
   * without this it would start the order again from the top: the busiest vaults would collect every coin of every run and the
   * tail would never get one. docs/phase11.md §7.
   */
  const cursorPath = `${DATA}/sim-launch-cursor.json`;
  let launched = 0;
  try { launched = Number((JSON.parse(readFileSync(cursorPath, "utf8")) as { launched: number }).launched) || 0; } catch { launched = 0; }

  await rehydrate().catch((err) => log("rehydrate-failed", { error: String((err as Error).message).slice(0, 200) }));

  const launchOne = async () => {
    const i = launched, { name, symbol } = coinName(i);
    const listing = order[i % Math.max(1, order.length)] ?? listings[i % listings.length];
    // a popular collection's coin behaves like one: the ranking a visitor sees is the catalogue's own, not a coin flip
    const mood: Mood = listing.weight >= 5 ? "runner" : listing.weight === 4 ? "momentum" : moodFor(i);
    const w = i % s.wallets.length;
    const out: Launched = await launch(s, w, d.launcher, d.ponsFactory, listing.id, { name, symbol, logo: logo(i), taxBps: taxFor(i) });
    creatorOf.set(out.coin.toLowerCase(), w);
    coins.push({ address: out.coin, curve: out.curve, router: out.router, listingId: listing.id, mood, phase: out.phase >= 1 ? "graduated" : "curve",
      bornMs: Date.now(), burstUntilMs: 0, bought: 0, sniped: i !== 0 });
    launched++;
    writeFileSync(cursorPath, JSON.stringify({ launched, note: "how far down the launch order this chain has got; a restart carries on from here" }) + "\n");
    tally.launches++;
    log("launched", { coin: out.coin, name, symbol, taxBps: taxFor(i), mood, listing: listing.id, into: listing.ref, weight: listing.weight, ofOrder: `${launched} of ${order.length}` });
  };

  const tradeOne = async () => {
    if (!coins.length) return;
    const c = coins[turn++ % coins.length];
    const now = Date.now();
    if (shouldSnipe(c, now)) {                       // the anti-snipe tax, once, so it can be seen in the fees
      c.sniped = true;
      if (await buy(s, turn + 3, c.curve, 2n * 10n ** 17n)) { tally.buys++; log("sniped", { coin: c.address, note: "Pons's decaying anti-snipe tax lands in the router" }); }
      return;
    }
    if (shouldBurst(c, now, rand())) c.burstUntilMs = burstEndsAt(now, rand());
    const step = nextStep(c, now, rand(), rand());
    if (step.kind === "skip") return;
    const { threshold, phase } = await phaseOf(s, d.ponsFactory, c.address);
    if (phase >= 1 && c.phase === "curve") { c.phase = "graduated"; tally.graduations++; log("graduated", { coin: c.address, note: "Pons moved it into its v4 pool; fees keep reaching the same router" }); }
    if (c.phase === "graduated") return;             // the pool needs a router path; the curve is where staging's fees come from
    const wei = BigInt(Math.max(1, Math.round(step.size * 1e6))) * threshold / 1_000_000n;
    if (step.kind === "buy") { if (await buy(s, turn, c.curve, wei > 0n ? wei : 10n ** 16n)) { tally.buys++; c.bought += step.size; } }
    else if (await sell(s, turn, c.address, c.curve, step.size * 4)) tally.sells++;
  };

  const harvestOne = async () => {
    if (!coins.length) return;
    const c = coins[Math.floor(rand() * coins.length)];
    const h = await harvest(s, turn + 7, c.router);   // a wallet with no connection to the coin: harvest is permissionless
    if (h && h.claimed > 0n) { tally.harvests++; log("harvested", { coin: c.address, claimed: h.claimed, toVault: h.toVault, toTreasury: h.toTreasury }); }
  };

  /**
   * docs/phase11.md §7. With 58 vaults and one simulator, a vault's own trading fees reach its cap in days, not minutes: the
   * product's whole middle — a cap met, a purchase announced, released, settled, distributed — would be visible on two vaults
   * and empty on the rest. So the simulator tops a vault up to just over its posted cap, through `depositFrom`, AS THE
   * TREASURY: the same call the treasury's twenty per cent arrives by, an address nobody holds a key to, crediting the coin
   * exactly as a real fee is credited. Every top-up is logged as a fixture. Nothing about the cap, the queue, the wait, the
   * bond or the settlement is faked by it; only the fees, which on a chain nobody trades are the one thing that cannot be real.
   *
   * It goes to whichever vault is NEAREST its cap, so the vault a visitor sees "saving for the next buy" is the one that fills.
   * Runes are never topped up: they have no cap to reach (docs/phase11.md §2).
   */
  let topUpAt = 0;
  const topUpOne = async () => {
    const withCoins = new Set(coins.map((c) => c.listingId));
    const eligible = listings.filter((l) => !l.isRune && withCoins.has(l.id));
    if (!eligible.length) return;
    const window = eligible.length <= 8 ? eligible : Array.from({ length: 8 }, (_x, i) => eligible[(topUpAt + i) % eligible.length]);
    topUpAt = (topUpAt + window.length) % eligible.length;
    let best: { l: SimListing; short: bigint; ratio: number } | null = null;
    for (const l of window) {
      const [cap, attributed, reserved] = await Promise.all([read<bigint>(l.vault, VAULT, "capWei"),
        read<bigint>(l.vault, VAULT, "attributed"), read<bigint>(l.vault, VAULT, "reserved")]);
      if (cap === 0n) continue;                                  // no cap posted yet: nothing to aim at
      const spendable = attributed > reserved ? attributed - reserved : 0n;
      const target = cap > MIN_DECLARATION ? cap : MIN_DECLARATION;     // the keeper declares at the cap, never under 0.05 ETH
      if (spendable >= target) continue;                         // it can already buy; the operator's next pass will
      const ratio = Number((spendable * 1000n) / target) / 1000;
      if (!best || ratio > best.ratio) best = { l, short: target - spendable + 10n ** 16n, ratio };
    }
    if (!best) return;
    // the coin that has actually traded, if any: the vault's queue charges the coin it was funded through, and a lot whose
    // coin nobody holds can never be drawn between. Failing that, any of the listing's coins — trading will reach it.
    const mine = coins.filter((c) => c.listingId === best!.l.id);
    const coin = mine.slice().sort((a, b) => b.bought - a.bought)[0];
    if (!coin) return;
    await s.fund(d.treasury, best.short + 10n ** 17n);
    await asRole(d.treasury, best.l.vault, encodeFunctionData({ abi: VAULT, functionName: "depositFrom", args: [coin.address as Hex] }), best.short);
    tally.toppedUp++;
    log("topped-up", { listing: best.l.id, ref: best.l.ref, coin: coin.address, wei: best.short, wasAt: `${Math.round(best.ratio * 100)}% of its cap`,
      note: "staging-fixture: fees no trade produced, arriving by the same call the treasury's share arrives by, so the vault credits them exactly as it credits real ones" });
  };

  /**
   * docs/phase11.md §7. What a spread pass may NOT do with 58 vaults is read every purchase and every lot of every one of them,
   * every twenty seconds, for ever: that cost grows with everything that has ever happened. Two things bound it.
   *
   *  1. A purchase that is settled, cancelled or written off, and a lot that is delivered, can never change again. Each is read
   *     once and remembered, so a pass costs what is OPEN, not what is over.
   *  2. The counts themselves are re-read for a rotating window of vaults (the first pass primes them all), because a new
   *     purchase only appears when the operator declares one, and noticing it a minute later costs nothing.
   */
  const purchaseMemo = new Map<string, PurchaseCensus>(), lotMemo = new Map<string, LotCensus>();
  const counts = new Map<number, { purchases: number; acquisitions: number }>();
  let windowAt = 0, primed = false, lastReads = 0;

  /** Read what the chain holds, hold back what would otherwise empty, and push the two states nothing else produces. */
  const spreadOnce = async () => {
    const purchases: PurchaseCensus[] = [], lots: LotCensus[] = [];
    const nowS = Math.floor(Date.now() / 1000);
    const STATUS = ["none", "declared", "cancelled", "released", "settled", "written-off"] as const;
    let reads = 0;
    const scan = !primed || listings.length <= SPREAD_WINDOW ? listings
      : Array.from({ length: SPREAD_WINDOW }, (_x, i) => listings[(windowAt + i) % listings.length]);
    windowAt = (windowAt + scan.length) % Math.max(1, listings.length);
    primed = true;
    for (const l of scan) {
      counts.set(l.id, { purchases: Number(await read<bigint>(l.vault, VAULT, "purchaseCount")), acquisitions: Number(await read<bigint>(l.vault, VAULT, "acquisitionCount")) });
      reads += 2;
    }
    for (const l of listings) {
      const n = counts.get(l.id);
      if (!n) continue;
      for (let i = 0; i < n.purchases; i++) {
        const key = `${l.vault.toLowerCase()}:${i}`, done = purchaseMemo.get(key);
        if (done) { purchases.push(done); continue; }
        const p = await read<{ status: number; deadline: bigint; returned: bigint; maxEth: bigint }>(l.vault, VAULT, "purchase", [BigInt(i)]);
        reads++;
        const st = STATUS[p.status] ?? "none";
        if (st === "none") continue;
        const entry: PurchaseCensus = { vault: l.vault, listingId: l.id, id: i, status: st as PurchaseCensus["status"],
          overdue: nowS > Number(p.deadline) && Number(p.deadline) > 0, returned: p.returned > 0n };
        // settled, cancelled and written off are final: nothing on chain can move them again
        if (st === "settled" || st === "cancelled" || st === "written-off") purchaseMemo.set(key, entry);
        purchases.push(entry);
      }
      for (let i = 0; i < n.acquisitions; i++) {
        const lotKey = `${l.id}:${i}`, doneLot = lotMemo.get(lotKey);
        if (doneLot) { lots.push(doneLot); continue; }
        const lot = await read<{ mode: number; stage: number; destinationAt: bigint; destination: string; winner: string }>(d.distributor, DIST, "lot", [BigInt(l.id), BigInt(i)]);
        reads++;
        const lotEntry: LotCensus = { listingId: l.id, acquisitionId: i, mode: lot.mode, stage: lot.stage, deliverable: lot.stage === 4 && lot.destination.length > 0 };
        // delivered, never to be distributed, or held: none of them moves again. A held lot could be chosen again by its
        // creator, but the simulator IS the creator here and it only ever chooses a lot that has no mode yet.
        if (lot.stage === 5 || lot.mode === 2 || lot.mode === 1) lotMemo.set(lotKey, lotEntry);
        lots.push(lotEntry);
        // the creator chooses, across every mode, before the operator's fallback fires and makes them all alike
        if (lot.mode === 0) {
          const a = await read<{ coin: string }>(l.vault, VAULT, "acquisition", [BigInt(i)]);
          const w = creatorOf.get(a.coin.toLowerCase());
          if (w !== undefined) {
            const mode = choiceFor(l.isRune, chosenCount);
            const named = mode === 5 ? s.wallets[(w + 1) % s.wallets.length].account.address : "0x0000000000000000000000000000000000000000";
            const { account, client } = s.wallets[w];
            await s.mined(await client.writeContract({ address: d.distributor as Hex, abi: DIST, functionName: "choose", account, chain: null, args: [BigInt(l.id), BigInt(i), mode, named as Hex] }));
            chosenCount++; tally.chosen++;
            log("creator-chose", { listing: l.id, acquisition: i, mode });
          }
        }
        // a winner who is one of ours posts an address; one is deliberately left undecided, so that state is always on screen
        if (lot.stage === 4 && !lot.destination) {
          const w = s.wallets.findIndex((x) => x.account.address.toLowerCase() === lot.winner.toLowerCase());
          const waiting = lots.filter((x) => x.stage === 4 && !x.deliverable).length;
          if (w >= 0 && waiting <= 1) {
            const { account, client } = s.wallets[w];
            await s.mined(await client.writeContract({ address: d.distributor as Hex, abi: DIST, functionName: "setDestination", account, chain: null,
              args: [BigInt(l.id), BigInt(i), taprootFor(l.id * 31 + i)] }));
            tally.destinations++;
            log("winner-posted-an-address", { listing: l.id, acquisition: i, address: "a BIP-350 test vector: nobody holds its key" });
          }
        }
      }
    }
    lastReads = reads;
    const { holds, pushes } = spread({ purchases, lots });
    writeFileSync(`${DATA}/sim-holds.json`, JSON.stringify(holds) + "\n");     // the operator reads this and leaves those alone
    for (const p of pushes) {
      try {
        if (p.type === "return-unspent") {
          // the purchaser hands part of a release back: the state a purchase reaches when the asset cost less than the cap allowed
          const pr = await read<{ maxEth: bigint; purchaser: string }>(p.vault, VAULT, "purchase", [BigInt(p.purchase)]);
          const back = (pr.maxEth * BigInt(Math.round(p.share * 100))) / 100n;
          await s.fund(pr.purchaser, back + 10n ** 17n);
          await asRole(pr.purchaser, p.vault, encodeFunctionData({ abi: VAULT, functionName: "returnUnspent", args: [BigInt(p.purchase)] }), back);
          tally.returned++;
          log("returned-unspent", { vault: p.vault, purchase: p.purchase, wei: back, note: "the asset cost less than the cap allowed" });
        } else if (p.type === "age-for-write-off") {
          // A receipt has three days to arrive; staging is reset long before that, so the state would never exist. One stored
          // deadline is rewritten to the past with the node's own method, exactly as the operator shortens the public waits.
          await ageDeadline(p.vault, p.purchase);
          log("aged-for-write-off", { vault: p.vault, purchase: p.purchase, note: "staging only: its three-day receipt deadline was moved into the past" });
        } else {
          const owner = await read<string>(d.registry, REGISTRY, "owner");
          await s.fund(owner, 10n ** 18n);
          await asRole(owner, p.vault, encodeFunctionData({ abi: VAULT, functionName: "writeOff", args: [BigInt(p.purchase), FIXTURE_REASON] }));
          tally.writtenOff++;
          log("written-off", { vault: p.vault, purchase: p.purchase, note: "its receipt never came; the bond pays the coins that were charged" });
        }
      } catch (err) { log("push-failed", { push: p.type, error: String((err as Error).message).slice(0, 200) }); }
    }
  };

  const report = () => {
    const byMood = Object.fromEntries(["steady", "momentum", "fading", "runner"].map((m) => [m, coins.filter((c) => c.mood === m).length]));
    log("minute", { coins: coins.length, launchedEver: launched, vaults: listings.length, withCoins: new Set(coins.map((c) => c.listingId)).size,
      graduated: coins.filter((c) => c.phase === "graduated").length, spreadReads: lastReads, byMood, ...tally });
    for (const k of Object.keys(tally) as (keyof typeof tally)[]) tally[k] = 0;
  };

  for (;;) {
    const now = Date.now();
    try {
      if (now >= nextListingsAt) { await loadListings(); nextListingsAt = Date.now() + LISTINGS_EVERY_MS; }
      // docs/phase11.md §7: the first two dozen coins come quickly so nobody meets an empty grid, then one every few minutes
      // until every vault has one, and only then the slow cadence Phase 8 chose for a chain that has to look like weeks of use.
      if (ON("STAGING_SIM_TRADE") && launched < MAX_COINS && now >= nextLaunchAt) {
        await launchOne();
        nextLaunchAt = Date.now() + nextLaunchDelayMs(launched, rand(), Math.min(24, listings.length), launched < listings.length ? 3 * 60_000 : 20 * 60_000);
      }
      else if (ON("STAGING_SIM_TRADE") && now >= nextTradeAt) { await tradeOne(); const c = coins[turn % Math.max(1, coins.length)]; nextTradeAt = Date.now() + globalGapMs(c ? gapMs(c, BASE_MS, Date.now(), rand()) : BASE_MS, coins.length); }
      if (ON("STAGING_SIM_HARVEST") && now >= nextHarvestAt) { await harvestOne(); nextHarvestAt = Date.now() + globalGapMs(HARVEST_EVERY_MS, coins.length, 5_000); }
      if (ON("STAGING_SIM_TOPUP") && now >= nextTopUpAt) { await topUpOne(); nextTopUpAt = Date.now() + TOPUP_EVERY_MS; }
      if (ON("STAGING_SIM_SPREAD") && now >= nextSpreadAt) { await spreadOnce(); nextSpreadAt = Date.now() + SPREAD_EVERY_MS; }
      if (now >= nextReportAt) { report(); nextReportAt = now + 60_000; }
      failures = 0;
    } catch (err) {
      tally.failures++;
      if (++failures >= MAX_CONSECUTIVE_FAILURES) { log("giving-up", { failures, error: String((err as Error).message).slice(0, 300) }); process.exit(1); }
      log("step-failed", { error: String((err as Error).message).slice(0, 200) });
      await sleep(2_000);
    }
    await sleep(150);
  }
}

if (import.meta.url === `file://${process.argv[1]}`) main().catch((err) => { log("fatal", { error: String((err as Error).message) }); process.exit(1); });
