import { readFileSync } from "node:fs";
import { Address, NETWORK, OutScript, TEST_NETWORK } from "@scure/btc-signer";
import { decodeFunctionResult, encodeFunctionData, parseAbi, type Hex } from "viem";
import { buyer, IC_HOST, type FrogRecV } from "./ic.js";
import { operatorIdentity } from "./identity.js";
import { feeBound, judge, keyPathSighash, leafOf, MAX_PURCHASE_WEI, MIN_PURCHASE_WEI, parsePsbt, proofFor, Refused, satsForWei, verifyProof, weiForSats, type Parsed } from "./judge.js";

/**
 * docs/v6/p1-services.md §2.2 (K4). The operator's offer tool: a venue's listing — the seller-signed PSBT a marketplace hands a
 * buyer — turned into `frog_offer(id, inscription, psbt, to_sign, proof)`, after EVERY check the canister's `frogflow::accept` and
 * `frogbuy::judge` make, in their order, against the same kinds of sources. It prints exactly what the canister would sign.
 * DRY RUN BY DEFAULT: only `--send`, with OPERATOR_IDENTITY_PEM set, calls `frog_offer`, and only when every check passed.
 *
 *   npm run offer -- --frog <id> --inscription <txid>i<n> (--psbt <hex|base64|@file> [--to-sign 1,2] | --unisat-bid <file.json>)
 *                    (--proof <file.json> | --members <file.json>) (--root 0x… | --stack <stack.json> [--rpc <url>])
 *                    --float <bc1p…> [--custody <bc1p…>] [--cap-wei <wei>] [--sats-per-eth <n>] [--limit-wei <wei>]
 *                    [--canister <id> --coll <n>] [--ic-host <url>] [--send]
 * Ordinals multi-collection (docs/v9/ordinals-collections.md §4): `--coll` is the books id of the frog's Inscriptions listing
 * (`frog_collections().coll`), required to read the frog's cap from the canister and to send.
 * The venue: any marketplace's PSBT (`--psbt`), or UniSat's `create_bid` answer saved to a file (`--unisat-bid`, the shape from
 * its swagger, UNVERIFIED like every UniSat shape here). The tool never calls a venue's bid endpoint itself: buying at UniSat needs
 * their written permission (docs/phase1.md F3), which the operator has asked for and not received.
 */

// ---- the facts the canister reads, as the tool reads them -------------------------------------------------------------------------

export type Location = { outpoint: string; offset: bigint; value: bigint };
export type Contents = { inscriptions: string[]; runes: string[]; value: bigint | null };
export type Explorer = { name: string; exists: boolean; confirmed: boolean; spent: boolean; value: bigint | null };
export type Check = { name: string; ok: boolean; detail: string };
export type OfferInput = {
  frogId: bigint; inscription: string; psbt: Uint8Array; toSign: number[]; proof: Hex[]; root: Hex; float: string; custody: string;
  capWei: bigint; satsPerEth: bigint; rateRef: string; limitWei: bigint | null; network?: "mainnet" | "testnet";
};
export type SignLine = { input: number; outpoint: string; valueSats: string; script: string; sighash: Hex };
export type OfferReport = {
  ok: boolean; frogId: string; inscription: string; checks: Check[]; refusal: string | null;
  seller: Location | null; spendSats: string | null; feeSats: string | null; feeBoundSats: string | null; capSats: string; costWei: string | null;
  inputs: { index: number; outpoint: string; valueSats: string | null; owner: string }[]; outputs: { index: number; valueSats: string; owner: string }[];
  willSign: SignLine[]; candid: string | null; toSign: number[]; proof: Hex[]; psbtHex: string;
};

const isInscriptionId = (s: string) => /^[0-9a-f]{64}i\d{1,10}$/.test(s);
const hexOf = (b: Uint8Array) => Buffer.from(b).toString("hex");
export const scriptOf = (addr: string, network: "mainnet" | "testnet" = "mainnet"): Uint8Array => OutScript.encode(Address(network === "mainnet" ? NETWORK : TEST_NETWORK).decode(addr));

/** Every check, in the canister's order, given the facts already read. Pure: the same facts give the same report. */
export function prepare(inp: OfferInput, loc: Location | null, contents: Contents | null, explorers: Explorer[]): OfferReport {
  const checks: Check[] = [];
  const add = (name: string, ok: boolean, detail: string) => { checks.push({ name, ok, detail }); return ok; };
  const capSats = satsForWei(inp.capWei, inp.satsPerEth);
  const base: OfferReport = { ok: false, frogId: String(inp.frogId), inscription: inp.inscription, checks, refusal: null, seller: loc, spendSats: null, feeSats: null,
    feeBoundSats: null, capSats: String(capSats), costWei: null, inputs: [], outputs: [], willSign: [], candid: null, toSign: inp.toSign, proof: inp.proof, psbtHex: hexOf(inp.psbt) };
  const stop = (why: string): OfferReport => ({ ...base, refusal: why });

  if (!add("inscription id", isInscriptionId(inp.inscription), inp.inscription)) return stop("the inscription id is not <txid>i<n>");
  // 1. membership
  if (!add("membership", verifyProof(inp.proof, inp.root, leafOf(inp.inscription)), `leafOf(${inp.inscription.slice(0, 12)}…) against the listing root ${inp.root}`))
    return stop("the proof does not verify against the listing's root");
  // 2. where it is: two providers agree on the location and on the output's contents
  if (!add("located", loc !== null, loc ? `${loc.outpoint} offset ${loc.offset}, ${loc.value} sats` : "the ordinals providers did not agree on where it is")) return stop("unlocated");
  if (!add("contents", !!contents && contents.inscriptions.length === 1 && contents.inscriptions[0] === inp.inscription && contents.runes.length === 0,
    contents ? `the seller's output carries ${JSON.stringify(contents.inscriptions)} and runes ${JSON.stringify(contents.runes)}` : "the providers did not agree on the output's contents"))
    return stop("the seller's output must carry exactly this inscription and no rune");
  const spent = explorers.some((e) => e.spent);
  if (!add("unspent", explorers.length >= 2 && !spent, `${explorers.map((e) => `${e.name}: ${e.exists ? (e.spent ? "spent" : "unspent") : "unknown"}`).join(", ")}`))
    return stop(spent ? "the seller's output is spent: the listing is gone" : "fewer than two explorers answered");
  const agree = explorers.every((e) => e.exists && e.confirmed && e.value === loc!.value) && (contents!.value === null || contents!.value === loc!.value);
  if (!add("confirmed at ord's value", agree, explorers.map((e) => `${e.name}: ${e.confirmed ? "confirmed" : "unconfirmed"}, ${e.value} sats`).join("; ")))
    return stop("the seller's output is not confirmed at ord's value on every explorer");
  // 3. the PSBT, judged against those facts
  let p: Parsed;
  try { p = parsePsbt(inp.psbt); } catch (err) { add("PSBT parses", false, String((err as Error).message).slice(0, 200)); return stop("the PSBT does not parse"); }
  const float = scriptOf(inp.float, inp.network), custody = scriptOf(inp.custody, inp.network);
  const owner = (s: Uint8Array | null) => (!s ? "unknown" : hexOf(s) === hexOf(float) ? "float (ours)" : hexOf(s) === hexOf(custody) ? "CUSTODY" : "other");
  base.inputs = p.inputs.map((i) => ({ index: i.index, outpoint: i.outpoint, valueSats: i.value === null ? null : String(i.value),
    owner: i.outpoint === loc!.outpoint ? "seller (the frog)" : owner(i.script) }));
  base.outputs = p.outputs.map((o) => ({ index: o.index, valueSats: String(o.value), owner: o.script[0] === 0x6a ? "OP_RETURN" : owner(o.script) }));
  let j;
  try { j = judge(p, inp.toSign, { float, custody, seller: loc!, capSats }); } catch (err) {
    add("judge", false, err instanceof Refused ? err.message : String((err as Error).message));
    return stop(`refused by the judge: ${(err as Error).message}`);
  }
  add("judge", true, `spend ${j.spend} sats, fee ${j.fee} (bound ${feeBound(j.spend)}), the frog lands in output ${j.custodyVout}`);
  base.spendSats = String(j.spend); base.feeSats = String(j.fee); base.feeBoundSats = String(feeBound(j.spend));
  // 4. the policy's limits, and the cap, at the rate (the canister reads its own: the oracle median)
  const cost = weiForSats(j.spend, inp.satsPerEth);
  base.costWei = String(cost);
  const lo = MIN_PURCHASE_WEI, hi = inp.limitWei ?? MAX_PURCHASE_WEI;
  if (!add("limits", cost >= lo && cost <= hi, `cost ${cost} wei (${inp.rateRef}) within [${lo}, ${hi}]`)) return stop("the cost is outside the purchase limits");
  if (!add("cap", cost <= inp.capWei, `cost ${cost} wei ≤ the frog's cap ${inp.capWei} wei${cost * 100n >= inp.capWei * 99n ? " — WITHIN 1%: the canister's own rate may refuse it" : ""}`)) return stop("the cost is over the frog's cap");
  // what the canister would sign: every float input (frogflow::accept signs all of `ours`), key path, SIGHASH_DEFAULT
  base.willSign = j.ours.map((i) => ({ input: i, outpoint: p.inputs[i].outpoint, valueSats: String(p.inputs[i].value), script: hexOf(p.inputs[i].script!), sighash: keyPathSighash(p, i) }));
  base.candid = `(${inp.frogId} : nat64, "${inp.inscription}", "${hexOf(inp.psbt)}", vec { ${inp.toSign.map((i) => `${i} : nat32`).join("; ")} }, vec { ${inp.proof.map((h) => `"${h.slice(2)}"`).join("; ")} })`;
  return { ...base, ok: true };
}

// ---- the venue's answer --------------------------------------------------------------------------------------------------------

export function bytesOf(s: string): Uint8Array {
  const t = s.trim();
  if (/^([0-9a-fA-F]{2})+$/.test(t)) return Uint8Array.from(Buffer.from(t, "hex"));
  if (/^[A-Za-z0-9+/=\s]+$/.test(t)) { const b = Buffer.from(t.replace(/\s+/g, ""), "base64"); if (b.length && b.subarray(0, 5).toString("hex") === "70736274ff") return Uint8Array.from(b); }
  throw new Error("the PSBT is neither hex nor base64 of a PSBT");
}

/** UniSat's `create_bid` answer (swagger shape, UNVERIFIED): `data.psbtBid` and `data.bidSignIndexes` (or `toSignInputs[].index`). */
export function fromUnisatBid(body: unknown): { psbt: Uint8Array; toSign: number[] } {
  const d = ((body as { data?: unknown })?.data ?? body) as { psbtBid?: unknown; bidSignIndexes?: unknown; toSignInputs?: unknown };
  if (typeof d?.psbtBid !== "string") throw new Error("not a UniSat bid: no data.psbtBid (the shape is from the swagger and unverified; nothing is guessed)");
  const idx = Array.isArray(d.bidSignIndexes) ? d.bidSignIndexes : Array.isArray(d.toSignInputs) ? (d.toSignInputs as { index?: unknown }[]).map((x) => x?.index) : null;
  if (!idx || !idx.every((x) => Number.isInteger(x) && (x as number) >= 0)) throw new Error("not a UniSat bid: no bidSignIndexes / toSignInputs");
  return { psbt: bytesOf(d.psbtBid), toSign: idx as number[] };
}

// ---- reading the facts: two ordinals providers, two explorers, 4663 for the root, the canister for the cap -------------------------

type Fetch = typeof fetch;
export async function explorersOf(outpoint: string, fetchImpl: Fetch = fetch, bases = ["https://mempool.space/api", "https://blockstream.info/api"]): Promise<Explorer[]> {
  const [txid, vout] = outpoint.split(":");
  const out: Explorer[] = [];
  for (const b of bases) {
    const name = new URL(b).host;
    try {
      const t = await fetchImpl(`${b}/tx/${txid}`, { headers: { "user-agent": "rook-ops-offer/1 (read-only)" } });
      if (t.status === 404) { out.push({ name, exists: false, confirmed: false, spent: false, value: null }); continue; }
      const tj = (await t.json()) as { vout?: { value?: number }[]; status?: { confirmed?: boolean } };
      const s = await fetchImpl(`${b}/tx/${txid}/outspend/${vout}`, { headers: { "user-agent": "rook-ops-offer/1 (read-only)" } });
      const sj = (await s.json()) as { spent?: boolean };
      const v = tj.vout?.[Number(vout)]?.value;
      out.push({ name, exists: v !== undefined, confirmed: tj.status?.confirmed === true, spent: sj.spent === true, value: v === undefined ? null : BigInt(v) });
    } catch { /* an explorer that does not answer is simply not counted: fewer than two refuses */ }
  }
  return out;
}

const LISTING = parseAbi(["function listing(uint256 id) view returns ((uint8 kind, bool paused, uint8 divisibility, address vault, bytes32 membershipRoot, string name, string ref, string btcVault))"]);
export async function listingRoot(rpcUrl: string, registry: Hex, listingId: bigint, fetchImpl: Fetch = fetch): Promise<Hex> {
  const data = encodeFunctionData({ abi: LISTING, functionName: "listing", args: [listingId] });
  const r = await fetchImpl(rpcUrl, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "eth_call", params: [{ to: registry, data }, "latest"] }) });
  const j = (await r.json()) as { result?: Hex; error?: { message: string } };
  if (!j.result) throw new Error(`the registry could not be read: ${j.error?.message ?? "no result"}`);
  return (decodeFunctionResult({ abi: LISTING, functionName: "listing", data: j.result }) as { membershipRoot: Hex }).membershipRoot;
}

export function wantedCap(frogs: FrogRecV[], id: bigint): bigint {
  const f = frogs.find((x) => x.id === id);
  if (!f) throw new Error(`frog ${id} is not in frog_buys()`);
  const w = (f.phase as { Wanted?: { cap_wei: bigint } }).Wanted;
  if (!w) throw new Error(`frog ${id} is ${Object.keys(f.phase)[0]}, not waiting for an offer`);
  return w.cap_wei;
}

// ---- the command ---------------------------------------------------------------------------------------------------------------

const arg = (argv: string[], name: string) => { const i = argv.indexOf(`--${name}`); return i >= 0 ? argv[i + 1] : undefined; };
const file = (v: string) => (v.startsWith("@") ? readFileSync(v.slice(1), "utf8") : v);

async function main(argv: string[]): Promise<number> {
  const need = (n: string) => { const v = arg(argv, n); if (v === undefined) throw new Error(`missing --${n}`); return v; };
  const send = argv.includes("--send");
  const collArg = arg(argv, "coll"), coll = collArg === undefined ? null : Number(collArg);
  if (coll !== null && (!Number.isInteger(coll) || coll < 0 || coll > 65_535)) throw new Error(`--coll ${collArg} is not a books id (0 to 65535)`);
  const needColl = (): number => { if (coll === null) throw new Error("missing --coll (the frog's collection, frog_collections().coll)"); return coll; };
  const frogId = BigInt(need("frog")), inscription = need("inscription");
  // the venue's listing
  let psbt: Uint8Array, toSign: number[];
  if (arg(argv, "unisat-bid")) ({ psbt, toSign } = fromUnisatBid(JSON.parse(readFileSync(arg(argv, "unisat-bid")!, "utf8"))));
  else { psbt = bytesOf(file(need("psbt"))); toSign = (arg(argv, "to-sign") ?? "").split(",").filter(Boolean).map(Number); }
  // the membership proof and the root it must verify against
  const proof: Hex[] = arg(argv, "proof") ? JSON.parse(readFileSync(arg(argv, "proof")!, "utf8")) : proofFor(JSON.parse(readFileSync(need("members"), "utf8")) as string[], inscription).proof;
  let root = arg(argv, "root") as Hex | undefined, custody = arg(argv, "custody");
  if (!root || !custody) {
    const stack = JSON.parse(readFileSync(need("stack"), "utf8")) as { registry: Hex; collections: { listingId: number; custody: { address?: string } }[] };
    const c = stack.collections[0];
    custody ??= c.custody.address;
    root ??= await listingRoot(arg(argv, "rpc") ?? "https://rpc.mainnet.chain.robinhood.com", stack.registry, BigInt(c.listingId));
  }
  if (!custody) throw new Error("no custody address: --custody or a stack file that names one");
  // the canister: the frog's cap (a query), and the rate
  const canister = arg(argv, "canister") ?? process.env.CANISTER_ID;
  const host = arg(argv, "ic-host") ?? process.env.IC_HOST ?? IC_HOST;
  let capWei = arg(argv, "cap-wei") ? BigInt(arg(argv, "cap-wei")!) : null;
  if (capWei === null) { if (!canister) throw new Error("--cap-wei, or --canister to read the frog's cap from frog_buys()"); capWei = wantedCap(await (await buyer(canister, { host })).frog_buys([needColl()]), frogId); }
  let satsPerEth: bigint, rateRef: string;
  if (arg(argv, "sats-per-eth")) { satsPerEth = BigInt(arg(argv, "sats-per-eth")!); rateRef = "the rate given with --sats-per-eth"; }
  else {
    const { productionRate } = await import("../../keeper/service/feed.js");
    satsPerEth = await productionRate(100n).satsPerEth(); rateRef = "the lower of Coinbase and Kraken, agreeing within 1% (the canister uses its own oracle median)";
  }
  // two ordinals providers (UniSat with UNISAT_API_KEY, ord, Ordinals Wallet) for the location and the contents; two explorers
  const { mainnetFeed } = await import("../../keeper/src/ordapi.js");
  const feed = mainnetFeed();
  const li = await feed.inscription(inscription);
  const lc = li.ok && li.value ? await feed.output(li.value.outpoint) : null;
  const contents = lc && lc.ok && lc.value ? { inscriptions: lc.value.inscriptions, runes: lc.value.runes, value: lc.value.value === null ? null : BigInt(lc.value.value) } : null;
  // ord's value of the output: the inscription read's, else the output read's (both are two providers agreeing); neither: unlocated
  const v = li.ok && li.value ? li.value.value ?? (contents?.value === null || contents === null ? null : Number(contents.value)) : null;
  const loc = li.ok && li.value && v !== null ? { outpoint: li.value.outpoint, offset: BigInt(li.value.offset), value: BigInt(v) } : null;
  const explorers = loc ? await explorersOf(loc.outpoint) : [];
  const report = prepare({ frogId, inscription, psbt, toSign, proof, root: root!, float: need("float"), custody, capWei, satsPerEth, rateRef, limitWei: arg(argv, "limit-wei") ? BigInt(arg(argv, "limit-wei")!) : null },
    loc, contents, explorers);
  if (!li.ok) report.checks.unshift({ name: "providers", ok: false, detail: `${li.why}: ${li.detail}` });
  console.log(JSON.stringify({ ...report, psbtHex: `${report.psbtHex.slice(0, 32)}… (${report.psbtHex.length / 2} bytes)` }, null, 1));
  if (!report.ok) { console.log(`REFUSED, nothing handed in: ${report.refusal}`); return 1; }
  if (!send) { console.log("DRY RUN: every check passed; nothing was handed in. Add --send (with OPERATOR_IDENTITY_PEM) to call frog_offer with the arguments above."); return 0; }
  const identity = operatorIdentity(process.env);
  if (!identity) { console.log("refused: --send needs OPERATOR_IDENTITY_PEM (frog_offer is operator-only)"); return 2; }
  if (!canister) { console.log("refused: --send needs --canister or CANISTER_ID"); return 2; }
  if (coll === null) { console.log("refused: --send needs --coll (the frog's collection, frog_collections().coll)"); return 2; }
  const b = await buyer(canister, { host, identity });
  const r = await b.frog_offer(coll, frogId, inscription, hexOf(psbt), toSign, proof.map((h) => h.slice(2)));
  console.log(JSON.stringify({ frog_offer: r }));
  return "Ok" in r ? 0 : 1;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main(process.argv.slice(2)).then((c) => process.exit(c), (err) => { console.error(`refused: ${String((err as Error).message).slice(0, 300)}`); process.exit(2); });
}
