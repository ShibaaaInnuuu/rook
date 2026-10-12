import type { Ev } from "../src/fold.js";

/** Every event the contracts can emit, in an order a real chain could produce. Invented values: used by the fold tests and by the SYNTHETIC fixtures. */
export const REG = "0x" + "11".repeat(20), LAUNCH = "0x" + "22".repeat(20), DIST = "0x" + "33".repeat(20), VAULT0 = "0x" + "a0".repeat(20), VAULT1 = "0x" + "a1".repeat(20), VAULT2 = "0x" + "a2".repeat(20);
/** the autonomous buyer's EVM address, for the one v2 listing in the story */
export const BUYER = "0x" + "b0".repeat(20);
export const COIN = "0x" + "c0".repeat(20), COIN2 = "0x" + "c1".repeat(20), ROUTER = "0x" + "d0".repeat(20), ROUTER2 = "0x" + "d1".repeat(20), CURVE = "0x" + "e0".repeat(20);
/** docs/phase10.md: the second coin's own curve, the PoolManager every pool shares, the Pons hook, and COIN's pool after it graduates. */
export const CURVE2 = "0x" + "e1".repeat(20), POOLS = "0x" + "b0".repeat(20), HOOK = "0x" + "b1".repeat(20), ZERO = "0x" + "00".repeat(20);
export const POOL_ID = "0x" + "7a".repeat(32);
export const WHO = "0x" + "f0".repeat(20), WINNER = "0x" + "f1".repeat(20), TX = "0x" + "99".repeat(32);
export const T0 = 1_800_000_000;
const B32 = (b: string) => "0x" + b.repeat(32);

export function story(): Ev[] {
  let block = 100, li = 0;
  const e = (contract: string, address: string, name: string, args: Ev["args"], dt = 12): Ev => ({ block: (block += 1), logIndex: li++ % 3, tx: TX, address, contract, name, args, ts: T0 + (block - 100) * dt });
  return [
    e("registry", REG, "Wired", { launcher: LAUNCH, distributor: DIST }),
    e("registry", REG, "NftRegistryWired", { nftRegistry: "0x" + "ab".repeat(20) }),
    e("registry", REG, "BuyerReserved", { buyer: "0x" + "ac".repeat(20) }),
    e("registry", REG, "RolesSet", { keeper: WHO, purchaser: WHO, attestor: WINNER }),
    e("registry", REG, "Listed", { id: "0", kind: 0, vault: VAULT0, name: "Stones", ref: "stones" }),
    e("registry", REG, "Listed", { id: "1", kind: 1, vault: VAULT1, name: "DOG", ref: "840000:3" }),
    // docs/v2/vault-v2.md: one v2 listing, so the fold that sets `autonomous` is exercised and not only declared. The pair
    // arrives in the order the contract emits it — `Listed` first, which IDENTITY_EVENTS also guarantees inside a block.
    e("registry", REG, "Listed", { id: "2", kind: 0, vault: VAULT2, name: "Frogs", ref: "bitcoin-frogs" }),
    e("registry", REG, "AutoBuyerNamed", { id: "2", buyer: BUYER, maxInFlight: "20000000000000000000" }),
    e("registry", REG, "Paused", { id: "1", paused: true }), e("registry", REG, "Paused", { id: "1", paused: false }),
    e("registry", REG, "RootProposed", { id: "0", root: B32("ab"), readyAt: "1800700000" }), e("registry", REG, "RootSet", { id: "0", root: B32("ab") }),
    e("registry", REG, "BtcVaultProposed", { id: "1", btcVault: "bc1pnew", readyAt: "1800700000" }), e("registry", REG, "BtcVaultSet", { id: "1", btcVault: "bc1pnew" }),
    e("registry", REG, "RolesProposed", { keeper: WHO, purchaser: WHO, attestor: WINNER, readyAt: "1800700000" }),
    e("registry", REG, "TreasuryProposed", { treasury: WHO, readyAt: "1800700000" }), e("registry", REG, "TreasurySet", { treasury: WHO }),
    e("registry", REG, "BondBpsProposed", { bps: 5000, readyAt: "1800700000" }), e("registry", REG, "BondBpsSet", { bps: 5000 }),
    e("launcher", LAUNCH, "Launched", { coin: COIN, creator: WHO, listingId: "0", curve: CURVE, router: ROUTER }),
    e("launcher", LAUNCH, "Launched", { coin: COIN2, creator: WHO, listingId: "1", curve: CURVE2, router: ROUTER2 }),
    /**
     * docs/phase13.md §2: the coin as an ERC-20. The mint — the whole supply, to the curve — then the curve selling some of
     * it, a wallet-to-wallet move that is NOT a trade, and a burn. COIN2 ends with one holder and two deleted rows, which is
     * the case a balance table gets wrong if it keeps holders of nothing.
     */
    e("coin", COIN, "Transfer", { from: ZERO, to: CURVE, value: "1000000" }),
    e("coin", COIN, "Transfer", { from: CURVE, to: WHO, value: "600000" }),
    e("coin", COIN, "Transfer", { from: WHO, to: WINNER, value: "100000" }),
    e("coin", COIN, "Transfer", { from: WINNER, to: ZERO, value: "50000" }),
    e("coin", COIN2, "Transfer", { from: ZERO, to: CURVE2, value: "900" }),
    e("coin", COIN2, "Transfer", { from: CURVE2, to: WHO, value: "900" }),
    e("coin", COIN2, "Transfer", { from: WHO, to: WINNER, value: "900" }),
    // docs/phase10.md: the coin's whole trading life — two trades on Pons's curve, the curve completing, then its v4 pool
    e("curve", CURVE, "CurveBuy", { buyer: WHO, recipient: WHO, quoteIn: "1000", tokensOut: "1000000", fee: "10", tax: "25" }),
    e("curve", CURVE, "CurveSell", { seller: WHO, recipient: WINNER, tokensIn: "500000", quoteOut: "460", fee: "5", tax: "12" }),
    e("curve", CURVE, "CurveCompleted", { recipient: WHO, quoteOut: "5000", tokenOut: "2000000" }),
    e("pool", POOLS, "Initialize", { id: POOL_ID, currency0: ZERO, currency1: COIN, fee: 3000, tickSpacing: 200, hooks: HOOK, sqrtPriceX96: "79228162514264337593543950336", tick: 0 }),
    e("pool", POOLS, "Swap", { id: POOL_ID, sender: WHO, amount0: "1000", amount1: "-1030000", sqrtPriceX96: "79228162514264337593543950336", liquidity: "1000000", tick: 0, fee: 3000 }),
    e("router", ROUTER, "TreasuryDeferred", { owed: "20" }),
    e("router", ROUTER, "Harvested", { claimed: "100", toVault: "80", toTreasury: "0" }),
    e("vault", VAULT0, "Deposited", { coin: COIN, amount: "80" }),
    e("router", ROUTER, "Harvested", { claimed: "50", toVault: "40", toTreasury: "30" }),
    e("vault", VAULT0, "Deposited", { coin: COIN, amount: "40" }),
    e("router", ROUTER, "TokenForwarded", { token: COIN2, treasury: WHO, amount: "7" }),
    e("vault", VAULT0, "BondPosted", { from: WHO, amount: "100" }),
    e("vault", VAULT0, "CapPosted", { capWei: "60", expiry: String(T0 + 9_000) }),
    e("vault", VAULT0, "Declared", { id: "0", maxEth: "60", quantity: "1", releasableAt: String(T0 + 3_900), capRef: "fixture:stones" }),
    e("vault", VAULT0, "Declared", { id: "1", maxEth: "30", quantity: "1", releasableAt: String(T0 + 3_900), capRef: "fixture:stones" }),
    e("vault", VAULT0, "Cancelled", { id: "1" }),
    e("vault", VAULT0, "Charged", { id: "0", coin: COIN, amount: "60" }),
    e("vault", VAULT0, "Released", { id: "0", purchaser: WHO, amount: "60", deadline: String(T0 + 300_000) }),
    e("vault", VAULT0, "Returned", { id: "0", amount: "5" }),
    e("vault", VAULT0, "Receipted", { id: "0", acquisitionId: "0", coin: COIN, assetRef: "abci0", amount: "1", btcTxid: B32("42"), satsPaid: "150000" }),
    e("vault", VAULT0, "Settled", { id: "0" }),
    e("vault", VAULT0, "BondWithdrawn", { to: WHO, amount: "40" }),
    e("vault", VAULT0, "Declared", { id: "2", maxEth: "20", quantity: "1", releasableAt: String(T0 + 3_900), capRef: "fixture:stones" }),
    e("vault", VAULT0, "Charged", { id: "2", coin: COIN, amount: "20" }),
    e("vault", VAULT0, "Released", { id: "2", purchaser: WHO, amount: "20", deadline: String(T0 + 300_000) }),
    e("vault", VAULT0, "BondSlashed", { id: "2", amount: "15" }),
    e("vault", VAULT0, "WrittenOff", { id: "2", amount: "5", reasonRef: B32("00") }),
    e("distributor", DIST, "Chosen", { listingId: "0", acquisitionId: "0", mode: 3, named: "0x" + "00".repeat(20) }),
    e("distributor", DIST, "DrawOpened", { listingId: "0", acquisitionId: "0", root: B32("cd"), totalTickets: "1000", uri: "ipfs://list", challengeEnds: String(T0 + 5_000) }),
    e("distributor", DIST, "DrawPinned", { listingId: "0", acquisitionId: "0", round: "777" }),
    e("distributor", DIST, "DrawResolved", { listingId: "0", acquisitionId: "0", winningTicket: "412" }),
    e("distributor", DIST, "Won", { listingId: "0", acquisitionId: "0", winner: WINNER }),
    e("distributor", DIST, "DestinationSet", { listingId: "0", acquisitionId: "0", destination: "bc1pfirst" }, 12),
    e("distributor", DIST, "DestinationSet", { listingId: "0", acquisitionId: "0", destination: "bc1pfixed" }),
    e("distributor", DIST, "DeliveryOverdue", { listingId: "0", acquisitionId: "0" }),
    e("distributor", DIST, "Delivered", { listingId: "0", acquisitionId: "0", txid: B32("77") }),
    // the rune listing: a receipt, pro rata, claims, a roll-over, a payout
    e("vault", VAULT1, "Declared", { id: "0", maxEth: "50", quantity: "900", releasableAt: String(T0 + 3_900), capRef: "fixture:dog" }),
    e("vault", VAULT1, "Receipted", { id: "0", acquisitionId: "0", coin: COIN2, assetRef: "840000:3", amount: "900", btcTxid: B32("55"), satsPaid: "90000" }),
    e("distributor", DIST, "Chosen", { listingId: "1", acquisitionId: "0", mode: 6, named: "0x" + "00".repeat(20) }),
    e("distributor", DIST, "DistributionOpened", { listingId: "1", acquisitionId: "0", root: B32("ef"), total: "900", uri: "ipfs://shares", challengeEnds: String(T0 + 5_000) }),
    e("distributor", DIST, "ShareClaimed", { listingId: "1", acquisitionId: "0", holder: WINNER, amount: "300" }),
    e("distributor", DIST, "ShareClaimed", { listingId: "1", acquisitionId: "0", holder: WHO, amount: "200" }),
    e("distributor", DIST, "MinimumPosted", { listingId: "1", units: "250", expiry: String(T0 + 90_000) }),
    e("distributor", DIST, "PayoutRequested", { payoutId: "0", listingId: "1", holder: WINNER, amount: "300", destination: "bc1pwinner" }),
    e("distributor", DIST, "PayoutRecorded", { payoutId: "0", txid: B32("88") }),
    e("distributor", DIST, "PayoutRequested", { payoutId: "1", listingId: "1", holder: WHO, amount: "250", destination: "bc1pwho" }),
    e("distributor", DIST, "PayoutOverdue", { payoutId: "1" }),
    e("distributor", DIST, "RolledOver", { listingId: "1", acquisitionId: "0", coin: COIN2, amount: "400" }),
    // a second inscription lot that is opened then cancelled, a third that is voided, a fourth that is held, a fifth named
    ...[1, 2, 3, 4].map((q) => e("vault", VAULT0, "Receipted", { id: "0", acquisitionId: String(q), coin: COIN, assetRef: `abci${q}`, amount: "1", btcTxid: B32("42"), satsPaid: "1" })),
    e("distributor", DIST, "DrawOpened", { listingId: "0", acquisitionId: "1", root: B32("cd"), totalTickets: "10", uri: "u", challengeEnds: String(T0 + 99_000) }),
    e("distributor", DIST, "DrawCancelled", { listingId: "0", acquisitionId: "1" }),
    e("distributor", DIST, "DrawOpened", { listingId: "0", acquisitionId: "2", root: B32("cd"), totalTickets: "10", uri: "u", challengeEnds: String(T0 + 5_000) }),
    e("distributor", DIST, "DrawPinned", { listingId: "0", acquisitionId: "2", round: "778" }), e("distributor", DIST, "DrawResolved", { listingId: "0", acquisitionId: "2", winningTicket: "9" }),
    e("distributor", DIST, "DrawVoided", { listingId: "0", acquisitionId: "2", unclaimedTicket: "9" }),
    e("distributor", DIST, "Chosen", { listingId: "0", acquisitionId: "3", mode: 1, named: "0x" + "00".repeat(20) }),
    e("distributor", DIST, "Chosen", { listingId: "0", acquisitionId: "4", mode: 5, named: WINNER }), e("distributor", DIST, "Won", { listingId: "0", acquisitionId: "4", winner: WINNER }),
    e("distributor", DIST, "DistributionOpened", { listingId: "1", acquisitionId: "9", root: B32("ef"), total: "1", uri: "u", challengeEnds: String(T0 + 5_000) }),
    e("distributor", DIST, "DistributionCancelled", { listingId: "1", acquisitionId: "9" }),
    // fixes-B-C §0: the keeper switch and the guardian, last so that no earlier event's block or time moves
    e("registry", REG, "GuardianSet", { guardian: WINNER }),
    e("registry", REG, "KeeperPaused", { by: WINNER, paused: true }), e("registry", REG, "KeeperPaused", { by: WHO, paused: false }),
  ];
}
