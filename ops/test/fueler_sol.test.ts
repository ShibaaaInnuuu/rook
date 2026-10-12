// docs/v9/fueler-solana.md and docs/v9/canister-only-solana.md §9.2: the fueler's Solana leg on the books home (FUEL_HOME=books: the
// canister's cranks pay the hot key, no program, no allocate), held against a fake world that runs Solana (the hot key, Relay's depository),
// the canister's treasury_status, a fake Relay API, Ethereum and the IC legs. The Relay judge is held to a real SOL → ETH quote
// (ops/rehearsal/relay-sol/fixtures/quotes/sol-eth-1.quote.json) and to the rules of autobuyer/canister-rs/src/relaysol.rs.
import assert from "node:assert/strict";
import { generateKeyPairSync, verify as edVerify } from "node:crypto";
import { mkdtempSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { base58 } from "@scure/base";
import { parseTransaction, keccak256, type Hex } from "viem";
import { Engine, MemoryJournal, type EngineCfg, type Journal, type State } from "../src/fuel/engine.js";
import type { FuelIo, Notify } from "../src/fuel/io.js";
import { fuelEvmSigner, fuelSolSigner, type SolSigner } from "../src/fuel/keys.js";
import { CKETH_FEE_WEI, CKETH_HELPER, IC, ICP_FEE_E8S, topUpAccount } from "../src/fuel/pinned.js";
import { decide, DEFAULT_SOL_CAPS, LAMPORTS, T, type Prices } from "../src/fuel/plan.js";
import { allowedSol, anchorDisc, compileLegacy, cuLimitIx, cuPriceIx, depositIx, findProgramAddress, FUND_WALLET, IX_DEPOSIT_NATIVE, judgeSolQuote,
  minOutFloor, orderIdAny, RELAY_SOL, SYSTEM_PROGRAM, transferIx, type Ix, type S3 } from "../src/fuel/sol.js";
import type { SolAccount, SolIo, SolStatus } from "../src/fuel/solio.js";
import { parseFuelerConfig } from "../src/fueler.js";
import { logger } from "../src/log.js";

const silent = logger(() => {});
const hex = (u: Uint8Array) => Buffer.from(u).toString("hex");
const FIX = new URL("../rehearsal/relay-sol/fixtures/", import.meta.url);
const fixture = () => JSON.parse(readFileSync(new URL("quotes/sol-eth-1.quote.json", FIX), "utf8"));

/** A fresh solana-keygen file (64 bytes: seed ‖ public key) in a temp dir; the seed never leaves the test. */
function keypairFile(): { path: string; address: string } {
  const { privateKey, publicKey } = generateKeyPairSync("ed25519");
  const seed = privateKey.export({ format: "der", type: "pkcs8" }).subarray(-32), pub = publicKey.export({ format: "der", type: "spki" }).subarray(-32);
  const dir = mkdtempSync(join(tmpdir(), "fueler-sol-")), path = join(dir, "hot.json");
  writeFileSync(path, JSON.stringify([...seed, ...pub]), { mode: 0o600 });
  return { path, address: base58.encode(Uint8Array.from(pub)) };
}
const KP = keypairFile();
const sol: SolSigner = fuelSolSigner({ FUELER_SOL_KEYPAIR_FILE: KP.path })!;
const HOT = sol.address;
const evm = fuelEvmSigner({ FUELER_EVM_KEY: "0x8f2a55949038a9610f50fb23b5883af3b4ecb3c3bb792cbcefbd1542c692be63" })!;
const ME = evm.address;
const PRINCIPAL = "ih4wp-rvfmw-iimsu-qnmdv-63mci-c6s5k-6dfao-fmrkp-3nsmu-mf5ul-hqe";
const BUYER = "ryjl3-tyaaa-aaaaa-aaaba-cai", TIMELOCK = "rrkah-fqaaa-aaaaa-aaaaq-cai";
const PAYEE = FUND_WALLET;
const T0 = 1_791_600_000;
const XDR = 22_322n;
const PRICES = { eth: 2495, icp: 3.15, sol: 110 };
const P: Prices = { ethUsd: PRICES.eth, icpUsd: PRICES.icp, solUsd: PRICES.sol, at: T0, sources: ["coingecko", "coinbase"] };
const S3_OK: S3 = { allocator: RELAY_SOL.ALLOCATOR, upgradeAuthority: RELAY_SOL.OWNER_AND_UPGRADE_AUTHORITY, deploySlot: RELAY_SOL.PROGRAM_DEPLOY_SLOT };

/** A Relay SOL → ETH quote for `hot` → `recipient` that hashes correctly: the real fixture with its addresses, amount, minimum and deadlines replaced. */
function solQuote(hot: string, recipient: string, lamports: bigint, nowS: number, edit: (o: any) => void = () => {}) { // eslint-disable-line @typescript-eslint/no-explicit-any
  const q = fixture(), o = q.protocol.v2.orderData, dl = nowS + 7 * 86_400;
  const fair = (lamports * BigInt(Math.round((PRICES.sol / PRICES.eth) * 1e12))) / 1_000n;   // wei
  o.inputs[0].payment.amount = lamports.toString();
  o.inputs[0].refunds[0].recipient = hot; o.inputs[0].refunds[1].recipient = recipient;
  for (const r of o.inputs[0].refunds) r.deadline = dl;
  o.output.payments[0].recipient = recipient;
  o.output.payments[0].expectedAmount = ((fair * 9_936n) / 10_000n).toString();
  o.output.payments[0].minimumAmount = ((fair * 9_886n) / 10_000n).toString();
  o.output.deadline = dl;
  q.protocol.v2.paymentDetails.amount = lamports.toString();
  edit(o);
  const id = orderIdAny(o);
  q.protocol.v2.orderId = id;
  const ix = q.steps[0].items[0].data.instructions[0];
  ix.keys[1].pubkey = hot; ix.keys[2].pubkey = hot;
  ix.data = hex(depositIx(hot, lamports, id).data);
  q.details.currencyOut.amount = o.output.payments[0].expectedAmount;
  return q;
}

// ---- 1. the encodings ----------------------------------------------------------------------------------------------------------------

test("sol encodings: PDAs and discriminators are the programs' own; the order id of a real SOL → ETH quote is Relay's", () => {
  assert.deepEqual(findProgramAddress([Buffer.from("vault")], RELAY_SOL.DEPOSITORY_PROGRAM), [RELAY_SOL.VAULT, 255]);
  assert.equal(findProgramAddress([Buffer.from("relay_depository")], RELAY_SOL.DEPOSITORY_PROGRAM)[0], RELAY_SOL.RELAY_DEPOSITORY, "an off-curve bump search");
  assert.deepEqual([...anchorDisc("global", "deposit_native")], [...IX_DEPOSIT_NATIVE]);
  const q = fixture();
  assert.equal(orderIdAny(q.protocol.v2.orderData), q.protocol.v2.orderId);
  // the instruction the fueler writes for the fixture's order is byte-identical to the quote's own
  const d = depositIx("5ecsyZA5SVEuTByKoDCUivha8uzAgeQJmgFnnvokFF21", 1_000_000_000n, q.protocol.v2.orderId);
  assert.equal(hex(d.data), q.steps[0].items[0].data.instructions[0].data);
  assert.deepEqual(d.keys.map((k) => [k.pubkey, k.signer, k.writable]), q.steps[0].items[0].data.instructions[0].keys.map((k: any) => [k.pubkey, k.isSigner, k.isWritable])); // eslint-disable-line @typescript-eslint/no-explicit-any
});

test("sol message: a legacy message puts the payer first, signers, then writable, then read-only; the hot key's signature verifies", () => {
  const bh = base58.encode(new Uint8Array(32).fill(7));
  const msg = compileLegacy(HOT, [cuLimitIx(100_000), depositIx(HOT, 5n, `0x${"ab".repeat(32)}`)], bh);
  assert.deepEqual([msg[0], msg[1], msg[2]], [1, 0, 4], "one signer; read-only: the state, system, compute budget, the depository");
  assert.equal(msg[3], 6, "six keys");
  assert.equal(base58.encode(msg.slice(4, 36)), HOT);
  assert.equal(base58.encode(msg.slice(36, 68)), RELAY_SOL.VAULT, "the one writable non-signer");
  const sig = sol.signMessage(msg);
  const spki = Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), Buffer.from(base58.decode(HOT))]);
  assert.ok(edVerify(null, Buffer.from(msg), { key: spki, format: "der", type: "spki" }, Buffer.from(sig)));
});

test("sol keys: the keypair file is read in keys.ts only; malformed, mismatched or doubly-set keys are refused without their content; the scrub removes it", () => {
  const dir = mkdtempSync(join(tmpdir(), "fueler-bad-"));
  const bad = join(dir, "bad.json");
  writeFileSync(bad, "[1,2,3]");
  assert.throws(() => fuelSolSigner({ FUELER_SOL_KEYPAIR_FILE: bad }), (e: Error) => /64 bytes/.test(e.message) && !e.message.includes("1,2,3"));
  const raw = JSON.parse(readFileSync(KP.path, "utf8")) as number[];
  const swapped = join(dir, "swapped.json");
  writeFileSync(swapped, JSON.stringify([...raw.slice(0, 32), ...raw.slice(32).map((x, i) => (i === 0 ? x ^ 1 : x))]));
  assert.throws(() => fuelSolSigner({ FUELER_SOL_KEYPAIR_FILE: swapped }), /does not match/);
  assert.throws(() => fuelSolSigner({ FUELER_SOL_KEYPAIR_FILE: KP.path, FUELER_SOL_KEYPAIR: JSON.stringify(raw) }), /not both/);
  assert.equal(fuelSolSigner({ FUELER_SOL_KEYPAIR: JSON.stringify(raw) })!.address, HOT, "the sealed-variable form");
  assert.equal(fuelSolSigner({}), null, "a dry run needs no key");
  const seedB58 = base58.encode(Uint8Array.from(raw.slice(0, 32))), full = base58.encode(Uint8Array.from(raw));
  assert.equal(sol.scrub(`x ${full} y ${seedB58} z ${JSON.stringify(raw)}`), "x [redacted] y [redacted] z [redacted]");
});

// ---- 2. the judge (S1–S12) and the allowlist ----------------------------------------------------------------------------------------

const WANT = (o: Partial<Parameters<typeof judgeSolQuote>[1]> = {}) => ({ hot: HOT, recipient: ME, lamports: LAMPORTS / 4n,
  minOutFloor: minOutFloor(LAMPORTS / 4n, PRICES.sol, PRICES.eth, 200), s3: S3_OK, nowS: T0, used: [] as string[], ...o });

test("judge: a real-shaped SOL → ETH quote passes S1–S12 at Relay's 0.5 % slippage and a 2 % oracle bound", () => {
  const j = judgeSolQuote(solQuote(HOT, ME, LAMPORTS / 4n, T0), WANT());
  assert.equal(j.lamports, LAMPORTS / 4n);
  assert.ok(j.minOut >= WANT().minOutFloor);
  // and the real fixture itself, at its own time, for its own addresses
  const q = fixture();
  assert.doesNotThrow(() => judgeSolQuote(q, { hot: "5ecsyZA5SVEuTByKoDCUivha8uzAgeQJmgFnnvokFF21", recipient: "0x9db3fb117ecfc8fc14ca263e401a4c57a6913d81",
    lamports: LAMPORTS, minOutFloor: 0n, s3: S3_OK, nowS: 1_792_170_370 - 7 * 86_400 + 60, used: [] }));
});

test("judge: every rule refuses its case, by name", () => {
  const L = LAMPORTS / 4n, ok = () => solQuote(HOT, ME, L, T0);
  const no = (q: unknown, re: RegExp, w = WANT()) => assert.throws(() => judgeSolQuote(q as Record<string, unknown>, w), re);
  const q1 = ok(); q1.steps.push(q1.steps[0]); no(q1, /rule S1/);
  const q2 = ok(); q2.steps[0].items[0].data.instructions.push(q2.steps[0].items[0].data.instructions[0]); no(q2, /rule S1/);
  const q3 = ok(); q3.steps[0].items[0].data.instructions[0].programId = SYSTEM_PROGRAM; no(q3, /rule S3/);
  no(ok(), /rule S3: the depository's allocator/, WANT({ s3: { ...S3_OK, allocator: HOT } }));
  no(ok(), /rule S3: the depository program was upgraded/, WANT({ s3: { ...S3_OK, deploySlot: S3_OK.deploySlot + 1n } }));
  no(ok(), /rule S3: the depository program was upgraded/, WANT({ s3: { ...S3_OK, upgradeAuthority: null } }));
  const q5 = ok(); q5.steps[0].items[0].data.instructions[0].keys[2].pubkey = PAYEE; no(q5, /rule S5/);          // credited to another depositor
  no(ok(), /rule S4/, WANT({ lamports: L + 1n }));
  const q6 = ok(); q6.protocol.v2.orderData.output.payments[0].recipient = "0x" + "66".repeat(20); no(q6, /rule S6/);   // a changed order no longer hashes
  no(solQuote(HOT, ME, L, T0, (o) => { o.inputs[0].refunds[0].recipient = PAYEE; }), /rule S8/);
  no(solQuote(HOT, ME, L, T0, (o) => { o.inputs[0].refunds = o.inputs[0].refunds.slice(1); }), /rule S8: no refund on solana/);
  no(solQuote(HOT, ME, L, T0, (o) => { o.inputs[0].refunds[1].recipient = "0x" + "66".repeat(20); }), /rule S8/);
  no(solQuote(HOT, ME, L, T0, (o) => { o.output.payments[0].recipient = "0x" + "66".repeat(20); }), /rule S9/);
  no(solQuote(HOT, ME, L, T0, (o) => { o.output.calls = ["0x12"]; }), /rule S9: the order carries calls/);
  no(solQuote(HOT, ME, L, T0, (o) => { o.output.payments[0].minimumAmount = ((BigInt(o.output.payments[0].minimumAmount) * 97n) / 100n).toString(); }), /rule S10/);
  no(solQuote(HOT, ME, L, T0, (o) => { o.output.deadline = T0 + 3_600; }), /rule S11/);
  const q12 = ok(); no(q12, /rule S12/, WANT({ used: [q12.protocol.v2.orderId] }));
});

const BOGUS = base58.encode(new Uint8Array(32).fill(9));
const CTX = { hot: HOT, payee: PAYEE, maxPriceMicro: 20_000n, maxUnits: 100_000 };

test("allowlist (Solana): only a Relay deposit credited to the hot key and a transfer to the cold payee (no program instruction: rook_core is retired)", () => {
  const id = `0x${"ab".repeat(32)}` as Hex;
  assert.equal(allowedSol([cuLimitIx(100_000), cuPriceIx(20_000n), depositIx(HOT, 5n, id)], CTX), "relay");
  assert.equal(allowedSol([transferIx(HOT, PAYEE, 1n)], CTX), "forward");
  const no = (ixs: Ix[]) => assert.throws(() => allowedSol(ixs, CTX), /refused/);
  no([transferIx(HOT, BOGUS, 1n)]);                                                   // SOL to anyone but the payee
  no([transferIx(HOT, PAYEE, 0n)]);
  no([{ program: BOGUS, keys: [{ pubkey: HOT, signer: false, writable: true }], data: anchorDisc("global", "allocate") }]);   // the retired allocate, on any program
  no([depositIx(PAYEE, 5n, id)]);                                                     // a deposit credited to another
  no([{ ...depositIx(HOT, 5n, id), keys: depositIx(HOT, 5n, id).keys.map((k, i) => (i === 2 ? { ...k, pubkey: PAYEE } : k)) }]);
  no([transferIx(HOT, PAYEE, 1n), transferIx(HOT, PAYEE, 1n)]);                       // two instructions
  no([cuPriceIx(20_001n), transferIx(HOT, PAYEE, 1n)]);                               // a fee over the cap
  no([cuLimitIx(100_001), transferIx(HOT, PAYEE, 1n)]);
  no([transferIx(HOT, PAYEE, 1n), cuLimitIx(1)]);                                     // the budget after the instruction
  no([{ program: BOGUS, keys: [], data: new Uint8Array(1) }]);
});
test("guard: the Solana key is read in keys.ts only; one signature site in engine.ts, after allowedSol, journaled before sendTransaction; raw sends only in solio.ts", () => {
  const dir = new URL("../src/", import.meta.url);
  const files = readdirSync(dir, { recursive: true }).map(String).filter((f) => f.endsWith(".ts"));
  const code = (f: string) => readFileSync(new URL(f, dir), "utf8");
  assert.deepEqual(files.filter((f) => /["']FUELER_SOL_KEYPAIR(_FILE)?["']|env\.FUELER_SOL_KEYPAIR|env\[\s*["']FUELER_SOL_KEYPAIR/.test(code(f))), ["fuel/keys.ts"]);
  assert.deepEqual(files.filter((f) => /\.signMessage\(/.test(code(f)) && f !== "fuel/keys.ts"), ["fuel/engine.ts"]);
  const e = code("fuel/engine.ts");
  assert.equal((e.match(/\.signMessage\(/g) ?? []).length, 1, "one Solana signing site");
  const send = e.slice(e.indexOf("private async sendSol"), e.indexOf("private async settleSol"));
  assert.ok(send.indexOf("allowedSol(") < send.indexOf("signMessage("), "the allowlist runs before the signature");
  assert.ok(send.indexOf("solSimulate(") < send.indexOf("signMessage("), "simulated before it is signed");
  assert.ok(send.indexOf("this.save()") < send.indexOf("solSend("), "journaled before it is sent");
  assert.deepEqual(files.filter((f) => /"sendTransaction"/.test(code(f))), ["fuel/solio.ts"]);
});

// ---- 3. the plan in SOL ----------------------------------------------------------------------------------------------------------------

test("decide (SOL): the run is the T caps' cycles at the oracle in lamports; 0.05 SOL of fees is short; the SOL day cap stops a run", () => {
  const base = { nowS: T0, lastRunAt: null, timelockCycles: null, opsWei: LAMPORTS, gasReserveWei: 1_000_000n, conversions: [], prices: P, xdrPermyriadPerIcp: XDR,
    caps: DEFAULT_SOL_CAPS, buyer: BUYER, timelock: null, asset: "SOL" as const };
  const d = decide({ ...base, buyerCycles: 34n * T });
  assert.equal(d.kind, "run");
  if (d.kind === "run") {
    const fair = (20e12 / Number((XDR * T) / 10_000n)) * PRICES.icp / PRICES.sol;    // SOL for 20 T at the oracle
    assert.ok(Number(d.wei) / 1e9 > fair && Number(d.wei) / 1e9 < fair * 1.06, `${d.wei} vs ${fair}`);
    assert.equal(d.expectCycles, 20n * T);
  }
  const short = decide({ ...base, buyerCycles: 34n * T, opsWei: LAMPORTS / 20n });
  assert.equal(short.kind === "blocked" && short.alert, "cycles-autofund-short");
  assert.match(short.kind === "blocked" ? short.why : "", /SOL/);
  const cap = decide({ ...base, buyerCycles: 34n * T, conversions: [{ at: T0 - 3_600, wei: LAMPORTS, cycles: 5n * T }] });
  assert.equal(cap.kind === "blocked" && cap.alert, "cycles-autofund-cap", "the day's 1 SOL is spent");
});

// ---- 4. a fake world -------------------------------------------------------------------------------------------------------------------

type Tx = { sigs: Uint8Array[]; msg: Uint8Array; keys: string[]; ixs: { program: string; accounts: string[]; data: Uint8Array }[] };
function decodeTx(raw: Uint8Array): Tx {
  let i = 0;
  const cu = () => { let v = 0, s = 0; for (;;) { const b = raw[i++]; v |= (b & 0x7f) << s; if (!(b & 0x80)) return v; s += 7; } };
  const sigs: Uint8Array[] = [];
  for (let n = cu(), k = 0; k < n; k++) { sigs.push(raw.slice(i, i + 64)); i += 64; }
  const start = i;
  i += 3;
  const keys: string[] = [];
  for (let n = cu(), k = 0; k < n; k++) { keys.push(base58.encode(raw.slice(i, i + 32))); i += 32; }
  i += 32;
  const ixs: Tx["ixs"] = [];
  for (let n = cu(), k = 0; k < n; k++) {
    const program = keys[raw[i++]];
    const accounts: string[] = []; for (let a = cu(), j = 0; j < a; j++) accounts.push(keys[raw[i++]]);
    const len = cu(); ixs.push({ program, accounts, data: raw.slice(i, i + len) }); i += len;
  }
  return { sigs, msg: raw.slice(start), keys, ixs };
}
const u64 = (d: Uint8Array, at: number) => Buffer.from(d).readBigUInt64LE(at);

class SolWorld implements FuelIo, SolIo {
  // Solana
  lamports = new Map<string, bigint>([[HOT, (LAMPORTS * 6n) / 10n]]);
  rent = 10_000_000n; height = 1_000; statuses = new Map<string, SolStatus>(); landed: Tx[] = []; sends = 0;
  drop = false; failNext = false; s3: S3 = S3_OK; crashAt: string | null = null; onCrash: (() => void) | null = null;
  relayRate = 0.993; relayPays = true; relayMinBps = 9_886n; quotes = 0;
  // Ethereum and the IC
  eth = 0n; ethNonce = 0; receipts = new Map<string, { ok: boolean; block: bigint }>(); helperDeposits = 0;
  ck = 0n; ckAllowance = 0n; unused = { icp: 0n, ck: 0n }; icp = 0n; icpBlocks: { to: string; e8s: bigint; key: string }[] = []; notified = new Map<number, bigint>();
  canisters = new Map<string, bigint>([[BUYER, 34n * T], [TIMELOCK, 2n * T]]);
  private crash(at: string) { if (this.crashAt === at) { this.crashAt = null; this.onCrash?.(); throw new Error(`CRASH at ${at}`); } }

  // ---- SolIo
  async solBalance(a: string) { return this.lamports.get(a) ?? 0n; }
  async solAccount(a: string, slice?: number): Promise<SolAccount | null> {
    if (a === RELAY_SOL.RELAY_DEPOSITORY) {
      const d = Buffer.alloc(106); Buffer.from([0x80, 0x35, 0x9c, 0xbd, 0xf3, 0x53, 0xf5, 0x6b]).copy(d, 0); Buffer.from(base58.decode(this.s3.allocator)).copy(d, 40);
      return { owner: RELAY_SOL.DEPOSITORY_PROGRAM, lamports: 1n, data: Uint8Array.from(d) };
    }
    if (a === RELAY_SOL.PROGRAMDATA) {
      const d = Buffer.alloc(45); d.writeUInt32LE(3, 0); d.writeBigUInt64LE(this.s3.deploySlot, 4);
      if (this.s3.upgradeAuthority) { d[12] = 1; Buffer.from(base58.decode(this.s3.upgradeAuthority)).copy(d, 13); }
      return { owner: RELAY_SOL.BPF_LOADER_UPGRADEABLE, lamports: 1n, data: Uint8Array.from(d.subarray(0, slice ?? 45)) };
    }
    if (this.lamports.has(a)) return { owner: SYSTEM_PROGRAM, lamports: this.lamports.get(a)!, data: new Uint8Array(0) };
    return null;
  }
  async solRent() { return this.rent; }
  async solBlockhash() { return { blockhash: base58.encode(new Uint8Array(32).fill(this.height % 250)), lastValidBlockHeight: this.height + 150 }; }
  async solBlockHeight() { return this.height; }
  async solStatus(sig: string) { return this.statuses.get(sig) ?? null; }
  async solSimulate(raw: Uint8Array) { const t = decodeTx(raw); return { err: this.effects(t, true), logs: [], units: 5_000 }; }
  async solSend(raw: Uint8Array) {
    this.sends++;
    this.crash("sol-send");
    const t = decodeTx(raw), sig = base58.encode(t.sigs[0]);
    if (this.drop || this.statuses.has(sig)) return;                        // dropped, or a rebroadcast of a landed one
    const spki = Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), Buffer.from(base58.decode(t.keys[0]))]);
    assert.ok(edVerify(null, Buffer.from(t.msg), { key: spki, format: "der", type: "spki" }, Buffer.from(t.sigs[0])), "a valid signature by the fee payer");
    const err = this.failNext ? { InstructionError: [2, "Custom"] } : this.effects(t, false);
    this.failNext = false;
    this.lamports.set(HOT, this.lamports.get(HOT)! - 5_000n - 2_000n);     // the fee and the priority fee
    this.statuses.set(sig, { err, confirmation: "finalized" });
    if (!err) this.landed.push(t);
  }
  /** what a transaction does (or, with `dry`, would do); an error string refuses it */
  effects(t: Tx, dry: boolean): string | null {
    const ix = t.ixs.at(-1)!, hot = t.keys[0];
    if (ix.program === SYSTEM_PROGRAM) {
      const v = u64(ix.data, 4);
      if ((this.lamports.get(hot) ?? 0n) < v + 7_000n) return "insufficient funds";
      if (!dry) { this.lamports.set(hot, this.lamports.get(hot)! - v); this.lamports.set(ix.accounts[1], (this.lamports.get(ix.accounts[1]) ?? 0n) + v); }
      return null;
    }
    if (ix.program === RELAY_SOL.DEPOSITORY_PROGRAM) {
      const v = u64(ix.data, 8);
      if ((this.lamports.get(hot) ?? 0n) < v + 7_000n) return "insufficient funds";
      if (!dry) {
        this.lamports.set(hot, this.lamports.get(hot)! - v);
        if (this.relayPays) this.eth += BigInt(Math.floor((Number(v) / 1e9) * (PRICES.sol / PRICES.eth) * this.relayRate * 1e18));
      }
      return null;
    }
    return "unknown program";
  }
  // ---- FuelIo (Ethereum and the IC; nothing on 4663)
  principal() { return PRINCIPAL; }
  async evmBalance(chain: number, a: Hex) { assert.equal(chain, 1, "nothing on 4663"); return a.toLowerCase() === ME ? this.eth : 0n; }
  async evmNonce(chain: number) { assert.equal(chain, 1); return this.ethNonce; }
  async evmReceipt(_c: number, h: Hex) { return this.receipts.get(h) ?? null; }
  async evmCall(): Promise<Hex> { throw new Error("no 4663 call on the Solana home"); }
  async evmCode() { return "0x" as Hex; }
  async evmBaseFee() { return 100_000_000n; }
  async evmEstimate() { return 50_000n; }
  async evmBroadcast(chain: number, raw: Hex) {
    assert.equal(chain, 1);
    const tx = parseTransaction(raw);
    if (tx.nonce! < this.ethNonce) return;
    this.ethNonce++;
    assert.equal(tx.to!.toLowerCase(), CKETH_HELPER);
    this.eth -= (tx.value ?? 0n) + 1_000_000_000_000n;
    this.ck += tx.value ?? 0n; this.helperDeposits++;
    this.receipts.set(keccak256(raw), { ok: true, block: 1n });
  }
  async relayQuote(body: Record<string, unknown>) {
    this.quotes++;
    assert.equal(body.originChainId, 792_703_809); assert.equal(body.destinationChainId, 1);
    return solQuote(String(body.user), String(body.recipient), BigInt(String(body.amount)), T0, (o) => {
      o.salt = `0x${this.quotes.toString(16).padStart(64, "0")}`;                     // every quote its own order (salt)
      o.output.payments[0].minimumAmount = ((BigInt(o.output.payments[0].minimumAmount) * this.relayMinBps) / 9_886n).toString();
    });
  }
  async prices() { return { cg: { ...PRICES }, cb: { ...PRICES } }; }
  async curves(): Promise<Hex[]> { throw new Error("no curves on the Solana home"); }
  async cycles(c: string) { return this.canisters.get(c)!; }
  /** the books home: the canister's treasury_status (its ops key and what its cranks paid ops so far) */
  books: { ops: string | null; totalToOps: bigint } | Error = { ops: HOT, totalToOps: 0n };
  async booksTreasury(c: string) { assert.equal(c, BUYER); if (this.books instanceof Error) throw this.books; return this.books; }
  async xdrRate() { return XDR; }
  async minterHelper() { return CKETH_HELPER; }
  async poolTokens() { return { token0: IC.ICP_LEDGER as string, token1: IC.CKETH_LEDGER as string }; }
  async ckBalance() { return this.ck; }
  async ckApprove(a: bigint) { this.ck -= CKETH_FEE_WEI; this.ckAllowance = a; }
  async poolUnused() { return { ...this.unused }; }
  async poolDeposit(a: bigint) { this.ck -= a + CKETH_FEE_WEI; this.ckAllowance -= a + CKETH_FEE_WEI; this.unused.ck += a; }
  quoteOf(ck: bigint) { return BigInt(Math.floor((Number(ck) / 1e18) * (PRICES.eth / PRICES.icp) * 1e8 * 0.997)); }
  async poolQuote(ck: bigint) { return this.quoteOf(ck); }
  async poolSwap(ck: bigint, min: bigint) { const out = this.quoteOf(ck); assert.ok(out >= min); this.unused.ck -= ck; this.unused.icp += out; return out; }
  async poolWithdraw(e8s: bigint) { this.unused.icp -= e8s; this.icp += e8s - ICP_FEE_E8S; }
  async icpBalance() { return this.icp; }
  async icpTransfer(to: Uint8Array, e8s: bigint, at: bigint) {
    const key = `${hex(to)}:${at}`, i = this.icpBlocks.findIndex((x) => x.key === key);
    if (i >= 0) return BigInt(i);
    this.icp -= e8s + ICP_FEE_E8S; this.icpBlocks.push({ to: hex(to), e8s, key });
    return BigInt(this.icpBlocks.length - 1);
  }
  async notifyTopUp(block: bigint, canister: string): Promise<Notify> {
    const b = this.icpBlocks[Number(block)];
    if (b.to !== hex(topUpAccount(canister))) return { err: "InvalidTransaction" };
    const got = this.notified.get(Number(block));
    if (got !== undefined) return { ok: got };
    const cycles = (b.e8s * XDR * T) / 10_000n / 100_000_000n;
    this.notified.set(Number(block), cycles); this.canisters.set(canister, this.canisters.get(canister)! + cycles);
    return { ok: cycles };
  }
}

const SOLCFG = { payee: PAYEE, forwardMinLamports: LAMPORTS / 100n, relayMaxFeeBps: 200, slippageBps: 50, priceMicro: 20_000n, units: 100_000 };
const CFG: EngineCfg = { mode: "send", caps: DEFAULT_SOL_CAPS, buyer: BUYER, timelock: TIMELOCK, treasury: "0x" as Hex, payee: "0x" as Hex,
  claimMinWei: 0n, allocateMinWei: 0n, ethMaxFeeWei: 50_000_000_000n, stuckS: 7_200, mintStuckS: 3_600, home: "books", sol: SOLCFG };

class CrashJournal implements Journal {
  inner = new MemoryJournal(); frozen = false;
  load(): State { return this.inner.load(); }
  save(s: State): void { if (!this.frozen) this.inner.save(s); }
}
function rig(w = new SolWorld(), cfg: Partial<EngineCfg> = {}, keys: { sol?: SolSigner | null; evm?: typeof evm | null } = {}) {
  const j = new CrashJournal();
  let now = T0;
  const make = () => new Engine({ io: w, signer: keys.evm === undefined ? evm : keys.evm, journal: j, cfg: { ...CFG, ...cfg }, log: silent, nowS: () => now,
    sol: { io: w, signer: keys.sol === undefined ? sol : keys.sol } });
  const box = { e: make() };
  w.onCrash = () => { j.frozen = true; };
  const restart = () => { j.frozen = false; box.e = make(); };
  const tick = async (n = 1, dt = 60) => { for (let i = 0; i < n; i++) { now += dt; await box.e.tick(); if (j.frozen) restart(); } };
  return { w, j, box, tick };
}
const deposits = (w: SolWorld) => w.landed.filter((t) => t.ixs.at(-1)!.program === RELAY_SOL.DEPOSITORY_PROGRAM).length;

// ---- 5. the Solana leg, end to end (the books home: the canister's cranks paid the hot key; nothing to allocate) -----------------------

test("a Solana run end to end: 34 T → about 20 T through one judged Relay deposit, the helper, the pool and the CMC; the income mark is the canister's", async () => {
  const w = new SolWorld();
  w.lamports.set(HOT, w.lamports.get(HOT)! + LAMPORTS / 10n);                   // the canister's TreasuryStep::Ops landed 0.1 SOL
  w.books = { ops: HOT, totalToOps: LAMPORTS / 10n };
  const { box, tick } = rig(w);
  assert.deepEqual(await box.e.preflight(), []);
  await tick(40);
  assert.ok(w.landed.every((t) => [RELAY_SOL.DEPOSITORY_PROGRAM, SYSTEM_PROGRAM].includes(t.ixs.at(-1)!.program)), "only Relay deposits and transfers: no program instruction");
  assert.equal(box.e.s.income.at(-1)?.totalToOps, String(LAMPORTS / 10n), "the income mark is the canister's total_to_ops");
  assert.equal(box.e.s.run, null, `run left open: ${JSON.stringify(box.e.s.history.at(-1))}`);
  assert.equal(box.e.s.history.at(-1)!.phase, "done", box.e.s.history.at(-1)!.note);
  const got = w.canisters.get(BUYER)! - 34n * T;
  assert.ok(got > 19n * T && got <= 21n * T, `the buyer got ${Number(got) / 1e12} T`);
  assert.equal(deposits(w), 1, "one Relay deposit");
  assert.equal(w.helperDeposits, 1, "one helper deposit");
  assert.equal(w.icpBlocks.length, 1);
  assert.equal(w.quotes, 1);
  const c = box.e.s.conversions[0];
  assert.ok(c.wei > LAMPORTS / 5n && c.wei < (LAMPORTS * 3n) / 10n, `the caps count lamports: ${c.wei}`);
  assert.deepEqual(Object.keys(box.e.s.alerts), []);
  assert.equal(box.e.s.sol.used.length, 1, "the order id is remembered (S12)");
  assert.equal(box.e.s.nonces[1], w.ethNonce, "the monitor's Ethereum nonce");
  assert.equal(box.e.s.nonces[4663], undefined, "nothing on 4663");
});

test("buffer and forward: everything above 0.05 SOL + a day's cap goes to the cold payee; nothing under the threshold", async () => {
  const w = new SolWorld();
  w.lamports.set(HOT, 3n * LAMPORTS);
  w.canisters.set(BUYER, 100n * T);
  const { tick } = rig(w);
  await tick();
  const keep = DEFAULT_SOL_CAPS.bufferWei + DEFAULT_SOL_CAPS.dayMaxWei;
  assert.equal(w.lamports.get(PAYEE), 3n * LAMPORTS - keep);
  assert.equal(w.lamports.get(HOT), keep - 7_000n, "the buffer and a day's cap stay (less the fee)");
  await tick(5);
  assert.equal(w.landed.length, 1, "nothing more: under the forward threshold");
});

test("a refused quote signs nothing: S10 (a minimum too far under the oracle), S3 (the depository upgraded); after 6 h the run gives up", async () => {
  const w = new SolWorld();
  w.relayMinBps = 9_600n;
  const { box, tick } = rig(w);
  await tick(3);
  assert.equal(deposits(w), 0);
  assert.match(box.e.s.run?.note ?? "", /rule S10/);
  w.relayMinBps = 9_886n; w.s3 = { ...S3_OK, deploySlot: S3_OK.deploySlot + 1n };
  await tick(1);
  assert.match(box.e.s.run?.note ?? "", /rule S3/);
  assert.equal(w.sends, 0, "nothing was ever sent");
  await tick(7, 3_600);
  assert.ok(box.e.s.history.some((h) => h.phase === "failed" && /gave up/.test(h.note ?? "")), JSON.stringify(box.e.s.history));
  assert.equal(box.e.s.conversions.length, 0, "nothing left the wallet: the caps are not charged");
  assert.equal(w.sends, 0);
});

test("an expired deposit (never landed) is redone with a fresh quote; it can never pay twice", async () => {
  const w = new SolWorld();
  w.drop = true;
  const { box, tick } = rig(w);
  await tick(2);
  assert.equal(box.e.s.run?.phase, "relay-sent");
  assert.equal(box.e.s.conversions.length, 1, "charged the moment it was signed");
  const first = box.e.s.sol.pending!.sig;
  await tick(3);
  assert.ok(w.sends >= 3, "the same bytes rebroadcast while its blockhash is valid");
  w.height += 200;                                                               // past its last valid height: it can never land
  await tick(1);
  assert.equal(box.e.s.sol.sent.at(-1)!.note, "expired");
  assert.equal(box.e.s.sol.sent.at(-1)!.sig, first);
  // in the same tick: the caps given back, a fresh quote, a fresh order, a new transaction (charged again)
  assert.equal(box.e.s.run?.phase, "relay-sent");
  assert.notEqual(box.e.s.sol.pending!.sig, first);
  assert.equal(box.e.s.conversions.length, 1);
  w.drop = false;
  await tick(40);
  assert.equal(box.e.s.history.at(-1)!.phase, "done", box.e.s.history.at(-1)!.note);
  assert.equal(deposits(w), 1);
  assert.notEqual(base58.encode(w.landed[0].sigs[0]), first, "a new transaction, a new order");
  assert.equal(box.e.s.sol.used.length, 2);
});

test("a deposit that fails on chain fails the run, gives the caps back and pages; a crash before the send rebroadcasts the same bytes", async () => {
  const w = new SolWorld();
  w.failNext = true;
  const a = rig(w);
  await a.tick(4);
  assert.equal(a.box.e.s.history.at(-1)?.phase, "failed");
  assert.equal(a.box.e.s.conversions.length, 0);
  assert.ok(a.box.e.s.alerts["cycles-autofund-failed"]);

  const w2 = new SolWorld();
  w2.crashAt = "sol-send";
  const b = rig(w2);
  await b.tick(40);
  assert.equal(w2.crashAt, null, "the crash happened");
  assert.equal(b.box.e.s.history.at(-1)!.phase, "done", b.box.e.s.history.at(-1)!.note);
  assert.equal(deposits(w2), 1, "one deposit");
  assert.equal(w2.quotes, 1, "the journaled transaction was sent again, never re-quoted");
});

test("dry run: the plan (the decision, the judged quote it would deposit, the forward) is computed and nothing is signed", async () => {
  const w = new SolWorld();
  w.lamports.set(HOT, 2n * LAMPORTS);
  const { box, tick } = rig(w, { mode: "dry", sol: { ...SOLCFG, watchAddress: HOT }, watchAddress: ME }, { sol: null, evm: null });
  const notes = await box.e.preflight();
  assert.ok(notes.some((n) => /dry run/.test(n)));
  await tick();
  assert.equal(w.sends, 0);
  assert.ok(box.e.plan.some((l) => l.startsWith("would-forward")), box.e.plan.join("\n"));
  await tick();
  const dep = box.e.plan.find((l) => l.startsWith("would-deposit"));
  assert.ok(dep && /relayOrder/.test(dep) && !/refused/.test(dep), box.e.plan.join("\n"));
  assert.ok(box.e.plan.some((l) => l.startsWith("decision") && /run: 0\.2/.test(l)), box.e.plan.join("\n"));
  assert.equal(w.sends, 0, "nothing signed or sent");
  assert.equal(box.e.s.run, null);
});

test("preflight: a journal from the other home is refused (the retired programs home's reads as the books home's); a payee that is the hot key is refused; a hot key that is not a plain wallet is refused", async () => {
  const w = new SolWorld();
  const a = rig(w, { home: "robinhood" });
  a.box.e.s.home = "robinhood"; a.j.save(a.box.e.s);
  const e = new Engine({ io: w, signer: evm, journal: a.j, cfg: CFG, log: silent, sol: { io: w, signer: sol } });
  await assert.rejects(() => e.preflight(), /belongs to the robinhood home/);
  // a journal the programs home wrote (home "solana", an allocate it sent) loads and is the books home's: the same hot key, the same unit
  const old = new MemoryJournal();
  old.save({ ...old.load(), home: "solana" as never, sol: { pending: null, sent: [{ sig: "oldAllocate", kind: "allocate", ok: true, at: T0 - 86_400 }], used: [] } });
  const e2 = new Engine({ io: w, signer: evm, journal: old, cfg: CFG, log: silent, sol: { io: w, signer: sol } });
  assert.deepEqual(await e2.preflight(), []);
  assert.equal(old.load().home, "books");
  assert.equal(old.load().sol.sent[0].kind, "allocate");
  await assert.rejects(() => rig(w, { sol: { ...SOLCFG, payee: HOT } }).box.e.preflight(), /hot key itself/);
  const w2 = new SolWorld();
  w2.solAccount = async (addr: string) => (addr === HOT ? { owner: BOGUS, lamports: 1n, data: new Uint8Array(8) } : null);
  await assert.rejects(() => rig(w2).box.e.preflight(), /not a plain wallet/);
});

// ---- 6. the books home's own: the canister's treasury_status, the configuration -----------------------------------------------------------

test("books home: preflight notes a canister whose treasury.ops is not the hot key, or an unreadable one; the ops-elsewhere note each tick", async () => {
  const w = new SolWorld();
  w.books = { ops: PAYEE, totalToOps: 0n };
  const a = rig(w);
  assert.ok((await a.box.e.preflight()).some((n) => /treasury_set_ops/.test(n)));
  await a.tick();
  assert.ok(a.box.e.plan.some((l) => l.startsWith("ops-elsewhere")), a.box.e.plan.join("\n"));
  w.books = { ops: null, totalToOps: 0n };
  assert.ok((await rig(w).box.e.preflight()).some((n) => /treasury.ops is unset/.test(n)));
  w.books = new Error("replica unreachable");
  assert.ok((await rig(w).box.e.preflight()).some((n) => /could not be read/.test(n)));
});

test("config: FUEL_HOME is books by default; it needs two Solana providers, not 4663's, and no program id; the payee defaults to the fund wallet", () => {
  const ENV = { FUELER_BUYER: BUYER, FUELER_RPC_SOL: "https://a.example,https://b.example",
    FUELER_RPC_1: "https://rpc.mevblocker.io,https://gateway.tenderly.co/public/mainnet" };
  const c = parseFuelerConfig(ENV);
  assert.equal(c.home, "books");
  assert.equal(c.engine.sol!.payee, FUND_WALLET);
  assert.equal(c.engine.caps.dayMaxWei, LAMPORTS);
  assert.equal(c.engine.caps.bufferWei, LAMPORTS / 20n);
  assert.equal(c.engine.mode, "dry");
  assert.equal(parseFuelerConfig({ ...ENV, FUELER_SOL_PAYEE: HOT, FUELER_SOL_DAY_MAX: "0.5" }).engine.caps.dayMaxWei, LAMPORTS / 2n);
  const bad = (e: Record<string, string | undefined>, re: RegExp) => assert.throws(() => parseFuelerConfig({ ...ENV, ...e }), re);
  bad({ FUELER_SOL_PROGRAM: BOGUS }, /books home has no rook_core/);
  bad({ FUELER_RPC_SOL: "https://a.example" }, /FUELER_RPC_SOL: at least two/);
  bad({ FUELER_SOL_PAYEE: "0xabc" }, /not a Solana address/);
  bad({ FUELER_SOL_BUFFER: "0.001" }, /FUELER_SOL_BUFFER must be above FUELER_SOL_FLOOR/);
  bad({ FUEL_HOME: "base" }, /neither books nor robinhood/);
  bad({ FUEL_HOME: "solana" }, /programs home is retired/);
  // --dry-run forces dry whatever FUELER_MODE says, and only it can assume a cycles balance
  const d = parseFuelerConfig({ ...ENV, FUELER_MODE: "send" }, { dryRun: true, assumeCyclesT: 34 });
  assert.equal(d.engine.mode, "dry");
  assert.equal(d.engine.assumeCycles, 34n * T);
  assert.equal(parseFuelerConfig(ENV, { assumeCyclesT: 34 }).engine.assumeCycles, undefined);
});
