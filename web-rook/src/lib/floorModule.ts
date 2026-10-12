import type { Leg, Venues } from "./quote";

/**
 * THE ADAPTER for the Floor Module (src/floor/FloorModule.sol as built; docs/v5/floor-module.md §6.1, §13.4, §13.6). Its twin
 * indexer/src/wrapped/floorModule.ts reads the reserves; this one prices a trade's ETH ↔ wX leg.
 *
 * What the module does: mints wX for ETH at `P_mint = max(price, target) × (1 + s_mint)` and buys wX back at
 * `P_burn = min(min(price, target) × (1 − s_burn), β)`, where `price()` is the fills-based price (the protocol's own fills, inside
 * ±fillBand of the keeper's accepted floor) and each side has its own spread (`spreads()`: the mint side carries the stress term).
 * Both sides are bounded by a leaky bucket per hour and by the caps (`mintCapacity()`, `burnCapacity()`), so a leg is quoted
 * only within the room the module has right now. `quoteMint` / `quoteBurn` are the contract's own answers; the arithmetic here is
 * theirs (`units = ethIn × 1e18 / unitPrice`, `eth = units × unitPrice / 1e18`), so a fixture and a live read agree to the wei.
 *
 * `ROUTER_USES_FLOOR_MODULE` is TRUE: EtchRouter as built takes the module in its constructor and, for every ETH ↔ wX leg, fills the
 * pool up to the module's price and the module for the rest (docs/v5/floor-module.md §8). A collection whose router was built
 * without a module (`module() == 0`) is handled by the caller passing `routable: false`.
 */
export const ROUTER_USES_FLOOR_MODULE = true;

export const I_FLOOR_MODULE = [
  { type: "function", name: "wrapped", stateMutability: "view", inputs: [], outputs: [{ type: "address" }] },
  // three words, not one: the accepted keeper floor, when it was posted, and its ttl
  { type: "function", name: "floor", stateMutability: "view", inputs: [], outputs: [{ type: "uint256", name: "priceWei" }, { type: "uint64", name: "postedAt" }, { type: "uint64", name: "ttl" }] },
  { type: "function", name: "fresh", stateMutability: "view", inputs: [], outputs: [{ type: "bool" }] },
  { type: "function", name: "price", stateMutability: "view", inputs: [], outputs: [{ type: "uint256" }] },
  { type: "function", name: "spreads", stateMutability: "view", inputs: [], outputs: [{ type: "uint256", name: "sMint" }, { type: "uint256", name: "sBurn" }] },
  { type: "function", name: "mintCapacity", stateMutability: "view", inputs: [], outputs: [{ type: "uint256", name: "unitPrice" }, { type: "uint256", name: "maxEthIn" }] },
  { type: "function", name: "burnCapacity", stateMutability: "view", inputs: [], outputs: [{ type: "uint256", name: "unitPrice" }, { type: "uint256", name: "maxUnits" }] },
  { type: "function", name: "quoteMint", stateMutability: "view", inputs: [{ type: "uint256", name: "ethIn" }], outputs: [{ type: "uint256" }] },
  { type: "function", name: "quoteBurn", stateMutability: "view", inputs: [{ type: "uint256", name: "units" }], outputs: [{ type: "uint256" }] },
  { type: "function", name: "ethBackedUnits", stateMutability: "view", inputs: [], outputs: [{ type: "uint256" }] },
  { type: "function", name: "escrowBalance", stateMutability: "view", inputs: [], outputs: [{ type: "uint256" }] },
  { type: "function", name: "mint", stateMutability: "payable", inputs: [{ type: "address", name: "to" }, { type: "uint256", name: "minUnits" }], outputs: [{ type: "uint256" }] },
  { type: "function", name: "burn", stateMutability: "nonpayable", inputs: [{ type: "uint256", name: "units" }, { type: "uint256", name: "minEth" }, { type: "address", name: "to" }], outputs: [{ type: "uint256" }] },
] as const;

const ONE = 10n ** 18n, BPS = 10_000n;
export type FloorModuleState = {
  /** `price()`: the fills-based mid the module prices at, wei per whole unit */
  priceWei: bigint;
  /** `spreads()`: each side's spread now, bps; the mint side includes the stress term */
  spreadMintBps: number; spreadBurnBps: number;
  /** `mintCapacity()` / `burnCapacity()`: the unit price each side charges now (0 when it cannot trade) and its room */
  mintPriceWei: bigint; mintRoomWei: bigint; burnPriceWei: bigint; burnRoomUnits: bigint;
};

/** A state from the pieces a fixture carries: the unit prices are the mid with the spread on, rounded as the contract rounds. */
export function stateFrom(m: { priceWei: bigint; spreadMintBps: number; spreadBurnBps: number; mintRoomWei: bigint; burnRoomUnits: bigint }): FloorModuleState {
  const mintPriceWei = (m.priceWei * (BPS + BigInt(m.spreadMintBps)) + BPS - 1n) / BPS;     // rounded up, against the minter
  const burnPriceWei = (m.priceWei * (BPS - BigInt(m.spreadBurnBps))) / BPS;                // rounded down, against the burner
  return { ...m, mintPriceWei, burnPriceWei };
}

/** Mint: ETH in, wX out at the mint price, only within the room: FloorModule.quoteMint, wei for wei. */
export function mintLeg(m: FloorModuleState, ethIn: bigint): Leg | null {
  if (m.priceWei <= 0n || m.mintPriceWei <= 0n || ethIn <= 0n || ethIn > m.mintRoomWei) return null;
  const out = (ethIn * ONE) / m.mintPriceWei;
  return { venue: "floorMint", tokenIn: "ETH", tokenOut: "WX", amountIn: ethIn, amountOut: out, mid: 1e18 / Number(m.priceWei),
    fees: [], spreadBps: (m.spreadMintBps / (10_000 + m.spreadMintBps)) * 10_000, contract: "FloorModule.mint, through EtchRouter (the pool first while it is cheaper)" };
}
/** Burn: wX in, ETH out at the burn price, only within the room: FloorModule.quoteBurn, wei for wei. */
export function burnLeg(m: FloorModuleState, unitsIn: bigint): Leg | null {
  if (m.priceWei <= 0n || m.burnPriceWei <= 0n || unitsIn <= 0n || unitsIn > m.burnRoomUnits) return null;
  const out = (unitsIn * m.burnPriceWei) / ONE;
  return { venue: "floorBurn", tokenIn: "WX", tokenOut: "ETH", amountIn: unitsIn, amountOut: out, mid: Number(m.priceWei) / 1e18,
    fees: [], spreadBps: m.spreadBurnBps, contract: "FloorModule.burn, through EtchRouter (the pool first while it pays more)" };
}

/** What the quote engine takes (`Venues.floor`), from a module's state, or null when the collection has none. */
export function floorVenue(m: FloorModuleState | null, routable = ROUTER_USES_FLOOR_MODULE): Venues["floor"] {
  if (!m) return null;
  return { mint: (e) => mintLeg(m, e), burn: (u) => burnLeg(m, u), routable, why: routable ? "" : "this collection's EtchRouter was built without the Floor Module; shown for comparison only" };
}
