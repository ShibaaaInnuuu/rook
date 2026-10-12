import { decodeEventLog, keccak256, parseAbi, toHex, type Hex } from "viem";
import type { Sim } from "./wallets.js";

/**
 * docs/phase8.md §2, §3. The one place the simulator sends a trade from. Launches go through OUR Launcher; buying and selling go
 * through Pons's REAL curve and, after graduation, its real pool. Nothing here is a fixture: these are the same calls a person
 * clicking the staging site makes.
 */
export const LAUNCHER = parseAbi([
  "struct Socials { string twitter; string telegram; string discord; string website; string farcaster; }",
  "struct LaunchParams { string name; string symbol; string logo; string description; Socials socials; uint16 creatorTaxBps; bytes32 expectedEconomics; bytes32 salt; uint256 launchConfigId; uint256 listingId; }",
  "function launch(LaunchParams p) payable returns (address coin, address curve, address router)",
  "event Launched(address indexed coin, address indexed creator, uint256 indexed listingId, address curve, address router)",
]);
export const FACTORY = parseAbi([
  "function launchFee() view returns (uint256)",
  "function getLaunchedToken(address token) view returns (address token_, address curve, address deployer, address creatorFeeRecipient, address pairToken, uint256 graduationThreshold, uint24 poolFee, int24 tickSpacing, uint16 creatorTaxBps, bool buybackEnabled, uint8 phase, uint256 sweptQuote, uint256 sweptTokens, uint256 sweptAt, bool exists)",
]);
export const CURVE = parseAbi([
  "function buy(uint256 quoteIn, uint256 minTokensOut, address recipient) payable returns (uint256)",
  "function sell(uint256 tokensIn, uint256 minQuoteOut, address recipient) returns (uint256)",
]);
export const ERC20 = parseAbi([
  "function approve(address spender, uint256 amount) returns (bool)",
  "function balanceOf(address owner) view returns (uint256)",
]);
export const ROUTER = parseAbi([
  "function harvest() returns (uint256 toVault, uint256 toTreasury)",
  "event Harvested(uint256 claimed, uint256 toVault, uint256 toTreasury)",
]);

export type Launched = { coin: Hex; curve: Hex; router: Hex; threshold: bigint; phase: number };

export async function launch(s: Sim, w: number, launcher: string, factory: string, listingId: number,
  p: { name: string; symbol: string; logo: string; taxBps: number }): Promise<Launched> {
  const { account, client } = s.wallets[w % s.wallets.length];
  const fee = (await s.pub.readContract({ address: factory as Hex, abi: FACTORY, functionName: "launchFee" })) as bigint;
  const hash = await client.writeContract({
    address: launcher as Hex, abi: LAUNCHER, functionName: "launch", value: fee, account, chain: null,
    args: [{ name: p.name, symbol: p.symbol, logo: p.logo, description: "A simulated coin on the etched staging chain. It is worth nothing.",
      socials: { twitter: "", telegram: "", discord: "", website: "", farcaster: "" }, creatorTaxBps: p.taxBps,
      expectedEconomics: `0x${"00".repeat(32)}` as Hex, salt: keccak256(toHex(`sim-${p.symbol}-${Date.now()}`)), launchConfigId: 0n, listingId: BigInt(listingId) }],
  });
  const r = await s.pub.waitForTransactionReceipt({ hash, pollingInterval: 400 });
  if (r.status !== "success") throw new Error("the launch reverted");
  const ev = r.logs.filter((l) => l.address.toLowerCase() === launcher.toLowerCase())
    .map((l) => decodeEventLog({ abi: LAUNCHER, data: l.data, topics: l.topics }))[0];
  const { coin, curve, router } = ev.args as unknown as { coin: Hex; curve: Hex; router: Hex };
  const t = (await s.pub.readContract({ address: factory as Hex, abi: FACTORY, functionName: "getLaunchedToken", args: [coin] })) as readonly unknown[];
  return { coin, curve, router, threshold: t[5] as bigint, phase: Number(t[10]) };
}

/** What Pons thinks of the coin now: its phase (the curve, or graduated into the pool) and how much of the curve is gone. */
export async function phaseOf(s: Sim, factory: string, coin: string): Promise<{ phase: number; threshold: bigint; sweptQuote: bigint }> {
  const t = (await s.pub.readContract({ address: factory as Hex, abi: FACTORY, functionName: "getLaunchedToken", args: [coin as Hex] })) as readonly unknown[];
  return { phase: Number(t[10]), threshold: t[5] as bigint, sweptQuote: t[11] as bigint };
}

export async function buy(s: Sim, w: number, curve: string, wei: bigint): Promise<boolean> {
  const { account, client } = s.wallets[w % s.wallets.length];
  await s.fund(account.address, wei + 10n ** 17n);
  const hash = await client.writeContract({ address: curve as Hex, abi: CURVE, functionName: "buy", value: wei, account, chain: null, args: [wei, 0n, account.address] });
  return (await s.mined(hash)).status === "success";
}

export async function sell(s: Sim, w: number, coin: string, curve: string, share: number): Promise<boolean> {
  const { account, client } = s.wallets[w % s.wallets.length];
  const held = (await s.pub.readContract({ address: coin as Hex, abi: ERC20, functionName: "balanceOf", args: [account.address] })) as bigint;
  const amount = (held * BigInt(Math.max(1, Math.round(share * 1000)))) / 1000n;
  if (amount === 0n) return false;
  await s.mined(await client.writeContract({ address: coin as Hex, abi: ERC20, functionName: "approve", account, chain: null, args: [curve as Hex, amount] }));
  const hash = await client.writeContract({ address: curve as Hex, abi: CURVE, functionName: "sell", account, chain: null, args: [amount, 0n, account.address] });
  return (await s.mined(hash)).status === "success";
}

/** `harvest()` takes no argument and anyone may call it, so a wallet with no connection to the coin does — repeatedly. */
export async function harvest(s: Sim, w: number, router: string): Promise<{ claimed: bigint; toVault: bigint; toTreasury: bigint } | null> {
  const { account, client } = s.wallets[w % s.wallets.length];
  try {
    const hash = await client.writeContract({ address: router as Hex, abi: ROUTER, functionName: "harvest", account, chain: null });
    const r = await s.pub.waitForTransactionReceipt({ hash, pollingInterval: 400 });
    if (r.status !== "success") return null;
    const ev = r.logs.filter((l) => l.address.toLowerCase() === router.toLowerCase())
      .map((l) => decodeEventLog({ abi: ROUTER, data: l.data, topics: l.topics }))[0];
    return ev ? (ev.args as unknown as { claimed: bigint; toVault: bigint; toTreasury: bigint }) : null;
  } catch { return null; }                 // nothing to harvest yet is the ordinary case, not a failure
}
