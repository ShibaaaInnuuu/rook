import { encodeAbiParameters, keccak256, type Abi, type Hex } from "viem";
import { READS } from "./abis.js";
import { tryRead, type Rpc } from "./rpc.js";

/** docs/phase4.md §2, "What is read from the chain besides logs". Live reads, cached, and never allowed to break an indexed answer. */
export type VaultLive = { balanceWei: string | null; attributedWei: string | null; waitingWei: string | null; reservedWei: string | null; spendableWei: string | null;
  bondWei: string | null; outstandingWei: string | null; requiredBondWei: string | null; capWei: string | null; capExpiry: number | null; blockedByPurchase: number | null;
  blocked: boolean | null; error: string | null };
export type CoinLive = { unspentWei: string | null; queued: boolean | null; graduated: boolean | null; escrowWei: string | null; routerBalanceWei: string | null;
  pool: null | { poolId: Hex; ethFeesWei: string; ethCreatorTaxWei: string; coinFees: string; coinCreatorTax: string; operatorOnly: boolean }; error: string | null };

const NATIVE = "0x0000000000000000000000000000000000000000";
/** Pons's v4 pool id for a graduated coin: keccak256(abi.encode(PoolKey)) with the native asset as currency0 (docs/phase1.md). */
export const poolIdOf = (coin: string, poolFee: number, tickSpacing: number, hook: string): Hex =>
  keccak256(encodeAbiParameters([{ type: "address" }, { type: "address" }, { type: "uint24" }, { type: "int24" }, { type: "address" }],
    [NATIVE as Hex, coin as Hex, poolFee, tickSpacing, hook as Hex]));

export class Live {
  private readonly cache = new Map<string, { at: number; value: unknown }>();
  private hook: string | null | undefined;
  private escrow: string | null | undefined;
  constructor(private readonly rpc: Rpc | null, private readonly ponsFactory: string | null, private readonly ttlMs = 15_000, private readonly now: () => number = Date.now) {}
  get available(): boolean { return this.rpc !== null; }

  private async cached<T>(key: string, make: () => Promise<T>): Promise<T> {
    const hit = this.cache.get(key);
    if (hit && this.now() - hit.at < this.ttlMs) return hit.value as T;
    const value = await make();
    this.cache.set(key, { at: this.now(), value });
    if (this.cache.size > 5_000) this.cache.clear();
    return value;
  }
  private big = async (to: string, fn: string, args: unknown[] = []): Promise<string | null> => {
    const v = await tryRead<bigint>(this.rpc!, to, READS as Abi, fn, args);
    return v === null ? null : String(v);
  };

  async vault(vault: string): Promise<VaultLive | null> {
    if (!this.rpc) return null;
    return this.cached(`vault|${vault}`, async () => {
      const [attributed, waiting, reserved, bond, outstanding, required, cap, expiry, blocked, balance] = await Promise.all([
        this.big(vault, "attributed"), this.big(vault, "waiting"), this.big(vault, "reserved"), this.big(vault, "bond"), this.big(vault, "outstanding"),
        this.big(vault, "requiredBond"), this.big(vault, "capWei"), this.big(vault, "capExpiry"),
        tryRead<readonly [boolean, bigint]>(this.rpc!, vault, READS as Abi, "isBlocked"),
        this.rpc!<Hex>("eth_getBalance", [vault, "latest"]).then((h) => String(BigInt(h))).catch(() => null)]);
      const failed = [attributed, reserved, cap, expiry].some((x) => x === null);
      return { balanceWei: balance, attributedWei: attributed, waitingWei: waiting, reservedWei: reserved,
        spendableWei: attributed !== null && reserved !== null ? String(BigInt(attributed) - BigInt(reserved)) : null, bondWei: bond, outstandingWei: outstanding,
        requiredBondWei: required, capWei: cap, capExpiry: expiry === null ? null : Number(expiry), blocked: blocked ? blocked[0] : null,
        blockedByPurchase: blocked && blocked[0] ? Number(blocked[1]) : null, error: failed ? "the node did not answer every read" : null } satisfies VaultLive;
    });
  }

  async coin(c: { coin: string; router: string; curve: string; vault: string; poolFee: number | null; tickSpacing: number | null }): Promise<CoinLive | null> {
    if (!this.rpc) return null;
    return this.cached(`coin|${c.coin}`, async () => {
      if (this.ponsFactory && this.hook === undefined) this.hook = (await tryRead<string>(this.rpc!, this.ponsFactory, READS as Abi, "memeHook"))?.toLowerCase() ?? null;
      if (this.ponsFactory && this.escrow === undefined) this.escrow = (await tryRead<string>(this.rpc!, this.ponsFactory, READS as Abi, "feeEscrow"))?.toLowerCase() ?? null;
      const [unspent, queued, graduated, escrow, balance] = await Promise.all([this.big(c.vault, "unspentOf", [c.coin]),
        tryRead<boolean>(this.rpc!, c.vault, READS as Abi, "queued", [c.coin]), tryRead<boolean>(this.rpc!, c.curve, READS as Abi, "graduated"),
        this.escrow ? this.big(this.escrow, "balanceOf", [c.router]) : Promise.resolve(null),
        this.rpc!<Hex>("eth_getBalance", [c.router, "latest"]).then((h) => String(BigInt(h))).catch(() => null)]);
      let pool: CoinLive["pool"] = null;
      if (graduated && this.hook && c.poolFee !== null && c.tickSpacing !== null) {
        const id = poolIdOf(c.coin, c.poolFee, c.tickSpacing, this.hook);
        const [ef, et, cf, ct] = await Promise.all([this.big(this.hook, "pendingFees", [id, NATIVE]), this.big(this.hook, "pendingCreatorTax", [id, NATIVE]),
          this.big(this.hook, "pendingFees", [id, c.coin]), this.big(this.hook, "pendingCreatorTax", [id, c.coin])]);
        if (ef !== null && et !== null && cf !== null && ct !== null) {
          // while ANY coin-denominated fee is pending only Pons's sweep operator may sweep the pool (docs/phase1.md)
          pool = { poolId: id, ethFeesWei: ef, ethCreatorTaxWei: et, coinFees: cf, coinCreatorTax: ct, operatorOnly: BigInt(cf) + BigInt(ct) > 0n };
        }
      }
      return { unspentWei: unspent, queued, graduated, escrowWei: escrow, routerBalanceWei: balance, pool,
        error: unspent === null || queued === null ? "the node did not answer every read" : null } satisfies CoinLive;
    });
  }
}
