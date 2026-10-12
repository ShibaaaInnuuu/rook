// BigInt port of Meteora DBC's tests/utils/create_curve.ts::designCurve (migration option 1 = DAMM v2).
// Usage: node design_curve.mjs [supply=1e9] [pctOnMigration=20] [thresholdSol=85] [migrationFeePct=50] [baseDecimals=6]
const MIN_SQRT = 4295048016n, MAX_SQRT = 79226673521066979257578248091n;
const [supplyUi = 1_000_000_000n, pct = 20n, thrSol = 85n, feePct = 50n, baseDec = 6n] = process.argv.slice(2).map(BigInt);
const quoteDec = 9n;
const isqrt = (n) => { if (n < 2n) return n; let x = n, y = (x + 1n) / 2n; while (y < x) { x = y; y = (x + n / x) / 2n; } return x; };
const ceilDiv = (a, b) => (a + b - 1n) / b;
const deltaBase = (lo, hi, L) => ceilDiv(L * (hi - lo), lo * hi);
const deltaQuote = (lo, hi, L, up) => up ? ceilDiv(L * (hi - lo), 1n << 128n) : (L * (hi - lo)) >> 128n;
const liqFromBase = (base, hi, lo) => (base * hi * lo) / (hi - lo);
const liqFromQuote = (quote, lo, hi) => (quote << 128n) / (hi - lo);
const totalSupply = supplyUi * 10n ** baseDec;
const Q = thrSol * 10n ** quoteDec;
const migBaseUi = supplyUi * pct / 100n;
// migration price (UI) = thr*(100-fee)/100 / migBaseUi ; raw = UI * 10^(quoteDec-baseDec)
const num = thrSol * (100n - feePct) * 10n ** (quoteDec - baseDec), den = 100n * migBaseUi;
const sqrtMig = isqrt((num << 128n) / den);
const migQuote = Q - Q * feePct / 100n;
const migBase = deltaBase(sqrtMig, MAX_SQRT, liqFromQuote(migQuote, MIN_SQRT, sqrtMig));
const swapAmount = totalSupply - migBase;
const sqrtStart = sqrtMig * migBase / swapAmount * 100n / (100n - feePct);
const L0 = [liqFromBase(swapAmount, sqrtMig, sqrtStart), liqFromQuote(Q, sqrtStart, sqrtMig)].reduce((a, b) => a < b ? a : b);
const curve = [{ sqrtPrice: sqrtMig, liquidity: L0 }];
// getTotalSupplyFromCurve
let next = sqrtStart; let tot = deltaQuote(next, curve[0].sqrtPrice, L0, true);
if (tot > Q) next = next + (Q << 128n) / L0; else next = curve[0].sqrtPrice;
const swapBase = deltaBase(sqrtStart, next, L0);
const maxOnCurve = deltaBase(sqrtStart, curve[0].sqrtPrice, L0);
const buf = swapBase + swapBase * 25n / 100n; const swapBuf = buf < maxOnCurve ? buf : maxOnCurve;
const migBase2 = deltaBase(next, MAX_SQRT, liqFromQuote(Q - Q * feePct / 100n, MIN_SQRT, next));
const minSupply = swapBuf + migBase2;
const remaining = totalSupply - minSupply;
if (remaining > 0n) curve.push({ sqrtPrice: MAX_SQRT, liquidity: liqFromBase(remaining, MAX_SQRT, sqrtMig) });
const priceUi = (s) => Number((s * s * 10n ** 18n) >> 128n) / 1e18 / 10 ** Number(quoteDec - baseDec);
const out = {
  supplyUi: supplyUi.toString(), baseDecimals: Number(baseDec), totalSupplyRaw: totalSupply.toString(),
  migrationQuoteThreshold: Q.toString(), migrationFeePercentage: Number(feePct),
  sqrtStartPrice: sqrtStart.toString(), curve: curve.map(c => ({ sqrtPrice: c.sqrtPrice.toString(), liquidity: c.liquidity.toString() })),
  startPriceSolPerToken: priceUi(sqrtStart), migrationPriceSolPerToken: priceUi(sqrtMig),
  startMcapSol: priceUi(sqrtStart) * Number(supplyUi), migrationMcapSol: priceUi(sqrtMig) * Number(supplyUi),
  swapBaseOnCurveRaw: swapBase.toString(), migrationBaseRaw: migBase.toString(), remaining: remaining.toString(),
};
console.log(JSON.stringify(out, null, 2));
