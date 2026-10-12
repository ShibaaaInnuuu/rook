// C5-I-1 (docs/v7/phase1-nft.md §5.1; README.md here): a READ-ONLY Relay quote for the operator's own small transfer, 4663 → Ethereum
// and Ethereum → 4663, native ETH both ways, and what the operator's own wallet will be asked to sign. Nothing is signed or sent here:
// POST /quote is a read of Relay's public API, and the user / recipient / refund address is the operator's PUBLIC address.
//
//   cd ops && node --import tsx rehearsal/relay/quote.ts --user 0x<your address> [--amount-wei 1000000000000000] [--dir both|out|back] [--json <dir>] [--force-refund]
//
// It checks what the canister's judge checks of a deposit (ops/src/relay.ts preJudge, adapted to an Ethereum output): one deposit step,
// to Relay's pinned depository 0x4cd0…bc31, depositNative(user, orderId) with the value; the order id recomputed from
// protocol.v2.orderData (hashStruct, orderIdOf) equal to the calldata's; the output paying the user in native ETH; the refunds.
import { writeFileSync } from "node:fs";
import { DEPOSIT_NATIVE, ETH_NATIVE, RELAY_API_DEFAULT, RELAY_DEPOSITORY_4663, orderIdOf } from "../../src/relay.js";

const argv = process.argv.slice(2);
const opt = (k: string, d?: string) => { const i = argv.indexOf(k); return i >= 0 ? argv[i + 1] : d; };
const user = opt("--user") ?? "";
if (!/^0x[0-9a-fA-F]{40}$/.test(user)) { console.error("refused: --user <your PUBLIC address> (the depositor, recipient and refund address); no key is ever asked"); process.exit(2); }
const amount = BigInt(opt("--amount-wei", "1000000000000000")!);
const dir = opt("--dir", "both")!;
const jsonDir = opt("--json");
// Relay's documented way to test a refund (docs.relay.link, API core concepts, Refunds): the referrer "debug-force-refund" makes the
// solver refund the deposit on the ORIGIN chain to refundTo instead of filling it
const forceRefund = argv.includes("--force-refund");
const API = process.env.RELAY_API ?? RELAY_API_DEFAULT;
const NAME: Record<number, string> = { 4663: "robinhood", 1: "ethereum" };
const eth = (w: bigint | string) => `${w} wei (${(Number(BigInt(w)) / 1e18).toFixed(9)} ETH)`;
const iso = (s: number | string) => new Date(Number(s) * 1000).toISOString();

async function quote(origin: number, destination: number): Promise<boolean> {
  const body = { user, originChainId: origin, destinationChainId: destination, originCurrency: ETH_NATIVE, destinationCurrency: ETH_NATIVE,
    recipient: user, tradeType: "EXACT_INPUT", amount: amount.toString(), refundTo: user,
    ...(forceRefund ? { referrer: "debug-force-refund" } : {}) };
  console.log(`\n=== ${NAME[origin]} (${origin}) → ${NAME[destination]} (${destination}), ${eth(amount)}, user = recipient = refundTo = ${user}${forceRefund ? "; FORCED REFUND (referrer debug-force-refund): expect a refund on the origin chain, no fill" : ""}`);
  const r = await fetch(`${API}/quote`, { method: "POST", headers: { "content-type": "application/json",
    // RELAY_API_KEY (the operator's, from the environment, never printed): /quote with the debug-force-refund referrer answers 401 without one
    ...(process.env.RELAY_API_KEY ? { "x-api-key": process.env.RELAY_API_KEY } : {}) }, body: JSON.stringify(body) });
  const q: any = await r.json(); // eslint-disable-line @typescript-eslint/no-explicit-any
  if (jsonDir) writeFileSync(`${jsonDir}/quote-${origin}-${destination}.json`, JSON.stringify(q, null, 1));
  if (!r.ok) { console.log(`Relay refused the quote: HTTP ${r.status} ${JSON.stringify(q).slice(0, 300)}`); return false; }
  const bad: string[] = [];
  const need = (c: boolean, why: string) => { if (!c) bad.push(why); };
  const steps = q.steps ?? [];
  console.log(`steps: ${steps.map((s: any) => `${s.id}/${s.kind} (${(s.items ?? []).length} item(s), requestId ${s.requestId})`).join("; ")}`); // eslint-disable-line @typescript-eslint/no-explicit-any
  need(steps.length === 1 && steps[0].id === "deposit" && steps[0].kind === "transaction" && steps[0].items?.length === 1, "not exactly one deposit transaction");
  const d = steps[0]?.items?.[0]?.data ?? {};
  console.log("WHAT YOUR WALLET SIGNS (one transaction; compare it field by field in the wallet before signing):");
  console.log(`  chainId ${d.chainId}  from ${d.from}\n  to      ${d.to}\n  value   ${eth(d.value ?? 0)}\n  data    ${d.data}`);
  if (d.gas || d.maxFeePerGas) console.log(`  gas ${d.gas ?? "?"}, maxFeePerGas ${d.maxFeePerGas ?? "?"}, maxPriorityFeePerGas ${d.maxPriorityFeePerGas ?? "?"}`);
  need(Number(d.chainId) === origin, `the deposit is for chain ${d.chainId}`);
  need(String(d.to ?? "").toLowerCase() === RELAY_DEPOSITORY_4663, `the deposit goes to ${d.to}, not Relay's depository ${RELAY_DEPOSITORY_4663} (the same address on both chains)`);
  need(BigInt(d.value ?? -1) === amount, `the deposit sends ${d.value}, not ${amount}`);
  const data = String(d.data ?? "").toLowerCase();
  need(data.length === 2 + 68 * 2 && data.startsWith(DEPOSIT_NATIVE), "the calldata is not depositNative(address, bytes32) (selector 0x49290c1c)");
  need(`0x${data.slice(34, 74)}` === user.toLowerCase(), "depositNative names another depositor");
  const id = `0x${data.slice(74)}`;
  const p = q.protocol?.v2;
  if (!p?.orderData) bad.push("no protocol.v2.orderData: nothing binds the deposit to a recipient");
  else {
    let computed = "";
    try { computed = orderIdOf(p.orderData); } catch (e) { bad.push(`the order cannot be hashed: ${(e as Error).message}`); }
    console.log(`order id: calldata ${id}; recomputed hashStruct(orderData) ${computed}; quote says ${p.orderId}`);
    need(computed === id && String(p.orderId).toLowerCase() === id, "the order id does not recompute to the calldata's");
    need(String(p.paymentDetails?.depository ?? "").toLowerCase() === RELAY_DEPOSITORY_4663, "paymentDetails names another depository");
    const o = p.orderData, out = o.output;
    console.log(`solver ${o.solver} (solverChainId ${o.solverChainId})`);
    for (const pay of out.payments ?? []) console.log(`output on ${out.chainId}: pays ${pay.recipient} expected ${eth(pay.expectedAmount)}, minimum ${eth(pay.minimumAmount)}; deadline ${iso(out.deadline)}`);
    need(out.chainId === NAME[destination] && out.payments?.length === 1 && String(out.payments[0].recipient).toLowerCase() === user.toLowerCase()
      && BigInt(out.payments[0].currency === "0x" ? 0 : out.payments[0].currency) === 0n, "the output is not one native-ETH payment to you on the destination");
    need((out.calls ?? []).length === 0, "the order makes calls");
    for (const i of o.inputs ?? []) for (const rf of i.refunds ?? [])
      console.log(`refund on ${rf.chainId}: to ${rf.recipient}, minimum ${eth(rf.minimumAmount)}, deadline ${iso(rf.deadline)}`);
    need((o.inputs ?? []).every((i: any) => (i.refunds ?? []).every((rf: any) => String(rf.recipient).toLowerCase() === user.toLowerCase())), "a refund goes to another address"); // eslint-disable-line @typescript-eslint/no-explicit-any
    console.log(`fees in the order: ${(o.fees ?? []).length}`);
  }
  const det = q.details ?? {};
  if (det.currencyOut) console.log(`Relay's details: out ${eth(det.currencyOut.amount)} (min ${eth(det.currencyOut.minimumAmount ?? "0")}); time estimate ${det.timeEstimate} s; total impact ${det.totalImpact?.percent ?? "?"} %`);
  for (const [k, f] of Object.entries(q.fees ?? {}) as [string, any][]) if (f?.amount && f.amount !== "0") console.log(`fee ${k}: ${eth(f.amount)}`); // eslint-disable-line @typescript-eslint/no-explicit-any
  console.log(bad.length ? `CHECKS FAILED — do NOT sign:\n  - ${bad.join("\n  - ")}` : "checks: ok (the depository, depositNative(you, orderId), the order id recomputed, the output and refunds to you)");
  return bad.length === 0;
}

const ok: boolean[] = [];
if (dir !== "back") ok.push(await quote(4663, 1));
if (dir !== "out") ok.push(await quote(1, 4663));
console.log("\nNothing was signed or sent. The quote's deposit expires with its order: quote again right before signing.");
process.exit(ok.every(Boolean) ? 0 : 1);
