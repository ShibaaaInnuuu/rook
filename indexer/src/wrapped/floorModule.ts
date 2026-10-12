import { parseAbi, type Abi } from "viem";
import type { Address, FloorModuleFill, FloorModuleRead, ProtocolLiquidityRead, TreasuryRead } from "../../types.js";
import { hex, read, type Rpc } from "../rpc.js";

/**
 * THE ADAPTER for the Floor Module (src/floor/FloorModule.sol, docs/v5/floor-module.md §13.6 "for the web") and, beside it, the
 * protocol treasury (src/floor/ProtocolTreasury.sol, §13.3) and the protocol-owned liquidity in the collection's lock
 * (src/pools/LiquidityLock.sol, §13.2). Signatures are copied from src/ as built: `floor()` answers three words (the accepted
 * keeper floor, when it was posted, its ttl), not one; `escrowBalance()` is both pots (B + S); `netUnits()` is the convertible
 * units only. Everything else reads the typed results. This file and its twin web/src/lib/floorModule.ts are the only two that
 * name the module's ABI.
 */
export const I_FLOOR_MODULE = parseAbi([
  "function buffer() view returns (uint256)",
  "function wrapped() view returns (address)",
  "function floor() view returns (uint256 priceWei, uint64 postedAt, uint64 ttl)",
  "function fresh() view returns (bool)",
  "function price() view returns (uint256)",
  "function fillBandBps() view returns (uint256)",
  "function fillCount() view returns (uint256)",
  "function fill(uint256 i) view returns ((uint192 price, uint64 at))",
  "function spreads() view returns (uint256 sMint, uint256 sBurn)",
  "function spreadBps() view returns (uint256)",
  "function ethBackedUnits() view returns (uint256)",
  "function reserveUnits() view returns (uint256)",
  "function netUnits() view returns (uint256)",
  "function floatUnits() view returns (uint256)",
  "function escrowBalance() view returns (uint256)",
  "function escrow() view returns (uint256)",
  "function reserve() view returns (uint256)",
  "function revenue() view returns (uint256)",
  "function betaConvertible() view returns (uint256)",
  "function betaReserve() view returns (uint256)",
  "function mintCapacity() view returns (uint256 unitPrice, uint256 maxEthIn)",
  "function burnCapacity() view returns (uint256 unitPrice, uint256 maxUnits)",
  "function quoteMint(uint256 ethIn) view returns (uint256)",
  "function quoteBurn(uint256 units) view returns (uint256)",
  "function loanOutstanding() view returns (uint256)",
  "function treasury() view returns (address)",
  "function volume7d() view returns (uint256)",
  // fixes-B-C B-02 / B-04: the reserve's surplus pot and the stale fallback; the registry the keeper is read from
  "function surplus() view returns (uint256)",
  "function escaped() view returns (bool)",
  "function registry() view returns (address)",
  // fixes-R3-contracts §1.2 / §2: the queue's refund hold; exitAtBacking() is escaped() alone since r4-contracts (A3-M1)
  "function exitAtBacking() view returns (bool)",
  "function holdsRefunds() view returns (bool)",
  // r4-contracts: a burn priced at all; the frozen floor a paused burn pays (at most the backing per unit); the grace after an unpause
  "function burnsOpen() view returns (bool)",
  "function pauseFloor() view returns (uint256)",
  "function RESUME_GRACE() view returns (uint256)",
  // docs/v6/relay-conversion.md §2: the advance — one unit's item budget paid out before its item arrives
  "function transitUnits() view returns (uint256)",
  "function advanced() view returns (uint256)",
  "function advancedAt() view returns (uint64)",
]) as Abi;
/** r4-contracts A3-L1: the registry's last real resume (ListingRegistry / IKeeperSource). */
export const I_KEEPER_SOURCE = parseAbi(["function keeperResumedAt() view returns (uint64)", "function keeperResumedAtBlock() view returns (uint64)",
  "function keeperPaused() view returns (bool)", "function keeper() view returns (address)"]) as Abi;
export const I_TREASURY = parseAbi([
  "function reservesPot() view returns (uint256)",
  "function floatsPot() view returns (uint256)",
  "function bootstrapPot() view returns (uint256)",
  "function unallocated() view returns (uint256)",
  "function reservesBps() view returns (uint16)",
  "function floatsBps() view returns (uint16)",
  "function bootstrapBps() view returns (uint16)",
  "function opsBps() view returns (uint16)",
  "function ops() view returns (address)",
  "function bootstrapLoan() view returns (uint256)",
  "function totalReceived() view returns (uint256)",
  "function totalAllocated() view returns (uint256)",
  "function totalToOps() view returns (uint256)",
  "function totalToReserves() view returns (uint256)",
  "function totalToFloats() view returns (uint256)",
  "function totalLent() view returns (uint256)",
  "function totalRepaid() view returns (uint256)",
  "function collectionCount() view returns (uint256)",
  "function collection(uint256 i) view returns (address)",
  "function weights() view returns (uint256[] w, uint256 total)",
  "function outstanding(address module) view returns (uint256)",
]) as Abi;
export const I_LOCK_PROTOCOL = parseAbi([
  "function PROTOCOL() view returns (address)",
  "function pendingEth(address coin) view returns (uint256)",
  "function pendingWrapped(address coin) view returns (uint256)",
  "function coinPositions(address coin) view returns (bytes32[])",
  "function sharedEthIn() view returns (uint256)",
  "function sharedWrappedIn() view returns (uint256)",
  "function feesInEth() view returns (uint256)",
  "function feesPaidEth() view returns (uint256)",
  "function treasury() view returns (address)",
]) as Abi;
const ZERO = "0x0000000000000000000000000000000000000000";
/** LiquidityLock.PROTOCOL: the pseudo-coin the protocol's own liquidity is booked under */
export const PROTOCOL = "0x0000000000000000000000000000000000000001";
const MAX_FILLS = 8;
const err = (e: unknown) => String((e as Error)?.message ?? e).slice(0, 160);

/** The registry a module reads its keeper from (fixes-B-C §0), or null for a module built before the fixes. */
export async function registryOf(rpc: Rpc, module: Address, block: number): Promise<Address | null> {
  try { const a = await read<string>(rpc, module, I_FLOOR_MODULE, "registry", [], hex(block)); return a.toLowerCase() === ZERO ? null : (a as Address); } catch { return null; }
}

/** `null` address: no module is deployed for this collection, so no unit is ETH-backed — a fact of the config, not a guess. */
export async function readFloorModule(rpc: Rpc | null, module: Address | null, wrapper: Address, block: number, at: number | null): Promise<FloorModuleRead> {
  if (!module) return { kind: "absent", note: "no Floor Module is deployed for this collection: every wX is backed by an item" };
  if (!rpc) return { kind: "error", address: module, error: "no chain to read" };
  try {
    const tag = hex(block);
    const r = <T,>(fn: string, args: unknown[] = []) => read<T>(rpc, module, I_FLOOR_MODULE, fn, args, tag);
    const [w, floor, fresh, price, band, fillCount, spreads, E, R, C, floatUnits, escrow, backing, reserve, revenue, bC, bR, mintCap, burnCap, loan, treasury, vol] = await Promise.all([
      r<string>("wrapped"), r<readonly [bigint, bigint, bigint]>("floor"), r<boolean>("fresh"), r<bigint>("price"), r<bigint>("fillBandBps"), r<bigint>("fillCount"),
      r<readonly [bigint, bigint]>("spreads"), r<bigint>("ethBackedUnits"), r<bigint>("reserveUnits"), r<bigint>("netUnits"), r<bigint>("floatUnits"),
      r<bigint>("escrowBalance"), r<bigint>("escrow"), r<bigint>("reserve"), r<bigint>("revenue"), r<bigint>("betaConvertible"), r<bigint>("betaReserve"),
      r<readonly [bigint, bigint]>("mintCapacity"), r<readonly [bigint, bigint]>("burnCapacity"), r<bigint>("loanOutstanding"), r<string>("treasury"), r<bigint>("volume7d")]);
    if (w.toLowerCase() !== wrapper.toLowerCase()) return { kind: "error", address: module, error: `the module wraps ${w}, not this collection's wrapper` };
    // fixes-B-C B-02 / B-04: a module built before the fixes has neither; the report then says "not read" rather than zero
    const [surplus, escaped, exitAtBacking, burnsOpen, pauseFloor, grace, reg] = await Promise.all([r<bigint>("surplus").catch(() => null), r<boolean>("escaped").catch(() => null),
      r<boolean>("exitAtBacking").catch(() => null), r<boolean>("burnsOpen").catch(() => null), r<bigint>("pauseFloor").catch(() => null),
      r<bigint>("RESUME_GRACE").catch(() => null), r<string>("registry").catch(() => null)]);
    // r4-contracts A3-L1: refunds held for RESUME_GRACE after the last unpause (the registry's keeperResumedAt)
    const resumed = grace !== null && reg ? await read<bigint>(rpc, reg as Address, I_KEEPER_SOURCE, "keeperResumedAt", [], tag).catch(() => null) : null;
    const heldUntil = resumed && grace !== null && Number(resumed) > 0 ? Number(resumed) + Number(grace) : null;
    // R4-check A4-I3: whether the keeper is down (paused, or none named) at this block, from the registry the module obeys; null: not read
    const [kPaused, kNow] = reg ? await Promise.all([read<boolean>(rpc, reg as Address, I_KEEPER_SOURCE, "keeperPaused", [], tag).catch(() => null),
      read<string>(rpc, reg as Address, I_KEEPER_SOURCE, "keeper", [], tag).catch(() => null)]) : [null, null];
    const keeperPaused = kPaused === true || (kNow !== null && kNow.toLowerCase() === ZERO) ? true : kPaused === false && kNow !== null ? false : null;
    // docs/v6/relay-conversion.md §2: a module built before the advance has none of the three; the report then leaves them null
    const [transit, adv, advAt, buffer] = await Promise.all([r<bigint>("transitUnits").catch(() => null), r<bigint>("advanced").catch(() => null),
      r<bigint>("advancedAt").catch(() => null), r<bigint>("buffer").catch(() => null)]);
    const n = Math.min(Number(fillCount), MAX_FILLS);
    const fills: FloorModuleFill[] = await Promise.all(Array.from({ length: n }, (_, i) => r<{ price: bigint; at: bigint }>("fill", [BigInt(i)]).then((f) => ({ priceWei: String(f.price), at: Number(f.at) }))));
    return { kind: "read", address: module,
      ethBackedUnits: String(E), reserveUnits: String(R), convertibleUnits: String(C), floatUnits: String(floatUnits),
      transitUnits: transit === null ? null : String(transit), bufferUnits: buffer === null ? null : String(buffer), advancedWei: adv === null ? null : String(adv), advancedAt: advAt === null ? null : Number(advAt),
      escrowWei: String(escrow), backingWei: String(backing), standingReserveWei: String(reserve), revenueWei: String(revenue),
      floorWei: String(floor[0]), floorPostedAt: Number(floor[1]), floorFresh: fresh, priceWei: String(price), fillBandBps: Number(band), fills,
      spreadMintBps: Number(spreads[0]), spreadBurnBps: Number(spreads[1]), betaConvertibleWei: String(bC), betaReserveWei: String(bR),
      mintPriceWei: String(mintCap[0]), mintRoomWei: String(mintCap[1]), burnPriceWei: String(burnCap[0]), burnRoomUnits: String(burnCap[1]),
      loanOutstandingWei: String(loan), treasury: treasury.toLowerCase() === ZERO ? null : (treasury as Address), volume7dWei: String(vol),
      surplusWei: surplus === null ? null : String(surplus), escaped, exitAtBacking, burnsOpen, pauseFloorWei: pauseFloor === null ? null : String(pauseFloor), keeperPaused,
      refundsHeldUntil: heldUntil !== null && at !== null && heldUntil > at ? heldUntil : null, at, block };
  } catch (e) { return { kind: "error", address: module, error: err(e) }; }
}

/** The protocol treasury (docs/v5/floor-module.md §13.3): its pots, shares, totals and every registered collection's weight and loan. */
export async function readTreasury(rpc: Rpc | null, treasury: Address | null, block: number, at: number | null): Promise<TreasuryRead> {
  if (!treasury) return { kind: "absent", note: "no protocol treasury is named: nothing funds reserves or floats, and no loan is owed" };
  if (!rpc) return { kind: "error", address: treasury, error: "no chain to read" };
  try {
    const tag = hex(block);
    const r = <T,>(fn: string, args: unknown[] = []) => read<T>(rpc, treasury, I_TREASURY, fn, args, tag);
    const [balance, rp, fp, bp, un, rb, fb, bb, ob, ops, loan, tr, ta, to, trs, tf, tl, trp, n, weights] = await Promise.all([
      rpc<string>("eth_getBalance", [treasury, tag]).then(BigInt), r<bigint>("reservesPot"), r<bigint>("floatsPot"), r<bigint>("bootstrapPot"), r<bigint>("unallocated"),
      r<number>("reservesBps"), r<number>("floatsBps"), r<number>("bootstrapBps"), r<number>("opsBps"), r<string>("ops"), r<bigint>("bootstrapLoan"),
      r<bigint>("totalReceived"), r<bigint>("totalAllocated"), r<bigint>("totalToOps"), r<bigint>("totalToReserves"), r<bigint>("totalToFloats"), r<bigint>("totalLent"), r<bigint>("totalRepaid"),
      r<bigint>("collectionCount"), r<readonly [readonly bigint[], bigint]>("weights")]);
    const modules = await Promise.all(Array.from({ length: Number(n) }, (_, i) => r<string>("collection", [BigInt(i)])));
    const owed = await Promise.all(modules.map((m) => r<bigint>("outstanding", [m])));
    return { kind: "read", address: treasury, balanceWei: String(balance),
      pots: { reservesWei: String(rp), floatsWei: String(fp), bootstrapWei: String(bp), unallocatedWei: String(un) },
      shares: { reservesBps: Number(rb), floatsBps: Number(fb), bootstrapBps: Number(bb), opsBps: Number(ob) },
      ops: ops.toLowerCase() === ZERO ? null : (ops as Address), bootstrapLoanWei: String(loan),
      totals: { receivedWei: String(tr), allocatedWei: String(ta), toOpsWei: String(to), toReservesWei: String(trs), toFloatsWei: String(tf), lentWei: String(tl), repaidWei: String(trp) },
      collections: modules.map((m, i) => ({ module: m as Address, weightWei: String(weights[0][i] ?? 0n), outstandingLoanWei: String(owed[i]) })),
      weightTotalWei: String(weights[1]), at, block };
  } catch (e) { return { kind: "error", address: treasury, error: err(e) }; }
}

/** The protocol's own positions in the collection's lock (docs/v5/floor-module.md §13.2): what is placed, what waits, what it earned. */
export async function readProtocolLiquidity(rpc: Rpc | null, lock: Address | null, block: number, at: number | null): Promise<ProtocolLiquidityRead> {
  if (!lock) return { kind: "absent", note: "no liquidity lock is named for this collection" };
  if (!rpc) return { kind: "error", lock, error: "no chain to read" };
  try {
    const tag = hex(block);
    const r = <T,>(fn: string, args: unknown[] = []) => read<T>(rpc, lock, I_LOCK_PROTOCOL, fn, args, tag);
    const [pe, pw, positions, se, sw, fi, fp, treasury] = await Promise.all([
      r<bigint>("pendingEth", [PROTOCOL]), r<bigint>("pendingWrapped", [PROTOCOL]), r<readonly string[]>("coinPositions", [PROTOCOL]),
      r<bigint>("sharedEthIn"), r<bigint>("sharedWrappedIn"), r<bigint>("feesInEth"), r<bigint>("feesPaidEth"), r<string>("treasury")]);
    return { kind: "read", lock, pendingEthWei: String(pe), pendingWrappedUnits: String(pw), positions: positions.length,
      sharedEthInWei: String(se), sharedWrappedInUnits: String(sw), feesInEthWei: String(fi), feesPaidEthWei: String(fp),
      treasury: treasury.toLowerCase() === ZERO ? null : (treasury as Address), at, block };
  } catch (e) { return { kind: "error", lock, error: err(e) }; }
}
