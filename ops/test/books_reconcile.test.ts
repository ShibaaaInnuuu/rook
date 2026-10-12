import assert from "node:assert/strict";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { backingStatus } from "../src/backing.js";
import { ata, decodeTokenAccount, DLMM_PROGRAM, listingOf, locator, NATIVE_MINT, pinRefs, readBooksReserves, rpcsOf, tokenAmount, type BooksReservesDeps, type ItemV, type ListingV } from "../src/booksreserves.js";
import { anchorDisc, b58 } from "../src/fuel/sol.js";
import { decodeBooksView, type BooksViewV, type CoinsCollectionV, type DeskViewV, type VaultV } from "../src/ic.js";
import { logger } from "../src/log.js";
import { combine, page, parseReconcileConfig, RH_RPC_KEYLESS, runOnce, served, SOLANA_CHAIN_ID, Store, type Channel } from "../src/reconcile.js";
import { solReader, type SolAccount, type SolReader } from "../src/solread.js";

/** W11: the proof of reserves on the books home, against the canister's OWN `books_view` bytes (autobuyer/canister-rs/tests/fixtures/views/books-view.hex,
 *  written by tests/desk_intents.rs: two items held, one ticket open, some SOL-backed units) and a fake chain at finalized. */
const FIX = readFileSync(new URL("../../autobuyer/canister-rs/tests/fixtures/views/books-view.hex", import.meta.url), "utf8").trim();
const WX = "DyRkUpQxYG2VnP2SkMdQs5BTsVPeKCpSHLxzByc2Sxvj"; // the fixture's wX mint (tests/desk_intents.rs)
const MODULE = "ModuLe1111111111111111111111111111111111111";
const VAULT = "VauLt11111111111111111111111111111111111111";
const mintBytes = (supply: bigint) => { const b = Buffer.alloc(82); b.writeBigUInt64LE(supply, 36); b[44] = 9; return Uint8Array.from(b); };
const tokenBytes = (amount: bigint) => { const b = Buffer.alloc(165); b.writeBigUInt64LE(amount, 64); return Uint8Array.from(b); };

function reader(accounts: Record<string, SolAccount | null>): SolReader {
  return {
    account: async (a) => accounts[a] ?? null,
    accounts: async (as) => as.map((a) => accounts[a] ?? null),
    programAccounts: async () => [],
    balance: async (a) => accounts[a]?.lamports ?? 0n,
    slot: async () => 1_234,
    rent: async () => 890_880n,
  };
}

function canister(view: BooksViewV, vaults: VaultV[], module = MODULE, desks: DeskViewV[] = []) {
  return {
    pool_desks: async () => desks,
    books_collections: async () => [[view.coll, view.wx_mint] as [number, string]],
    books_view: async (c: number) => (c === view.coll ? [view] as [BooksViewV] : [] as []),
    coins_collections: async () => [{ index: view.coll, symbol: "wMILADY", stage: "Open", wx_mint: view.wx_mint, config: "", module, coins: 0, backed: 0 } as CoinsCollectionV],
    coins_vaults: async () => vaults,
  };
}

test("W11: the canister's books_view bytes decode and reconcile green against a chain that matches them", async () => {
  const v = decodeBooksView(FIX)[0]!;
  assert.equal(v.coll, 0);
  assert.equal(v.wx_mint, WX);
  assert.equal(v.held, 2n);
  assert.equal(v.open_tickets, 1n);
  assert.equal(v.items.length, 2);
  const expected = v.expected_supply[0]!;
  assert.equal(expected, 1_000_000_000n + v.sol_backed);
  const accounts: Record<string, SolAccount> = {
    [WX]: { owner: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", lamports: 1_461_600n, data: mintBytes(expected) },
    [MODULE]: { owner: "11111111111111111111111111111111", lamports: 890_880n + v.owed + 5n, data: new Uint8Array() },
    [VAULT]: { owner: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", lamports: 2_039_280n, data: tokenBytes(2_500_000_000n) },
  };
  const vaults: VaultV[] = [{ coll: 0, coin: "CoinMint", pool: "", stage: "Backed", vault: [VAULT], wx_in: 2_500_000_000n, coins_burned: 0n, wx_paid: 0n, backed_at: 1n }];
  const locate = async () => ({ where: "custody" as const });
  const r = await readBooksReserves({ reader: reader(accounts), canister: canister(v, vaults), locate, nowS: 1_700_000_000 });
  assert.equal(r.collections.length, 1);
  const c = r.collections[0];
  assert.deepEqual([c.status, c.reasons], ["green", []], JSON.stringify(c.reasons));
  assert.equal(c.equation.holds, true);
  assert.equal(c.equation.itemsInCustody, 2);
  assert.equal(c.equation.ticketsInFlight, 1);
  assert.equal(c.coinVaults[0].onChain, "2500000000");
  assert.equal(r.backing.kind, "read");
  assert.equal(backingStatus(r.backing, 1_700_000_000).status, "green");
  assert.equal(tokenAmount(tokenBytes(7n)), 7n);
});

test("W11: a supply off by one, a module short of its books, an item elsewhere and a short coin vault are each RED; an unreadable mint is UNVERIFIED", async () => {
  const v = decodeBooksView(FIX)[0]!;
  const expected = v.expected_supply[0]!;
  const base = (over: Partial<Record<string, SolAccount | null>>): Record<string, SolAccount | null> => ({
    [WX]: { owner: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", lamports: 1n, data: mintBytes(expected) },
    [MODULE]: { owner: "11111111111111111111111111111111", lamports: 890_880n + v.owed, data: new Uint8Array() },
    [VAULT]: { owner: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", lamports: 1n, data: tokenBytes(2_500_000_000n) },
    ...over,
  });
  const vaults: VaultV[] = [{ coll: 0, coin: "CoinMint", pool: "", stage: "Backed", vault: [VAULT], wx_in: 2_500_000_000n, coins_burned: 0n, wx_paid: 0n, backed_at: 1n }];
  const inCustody: BooksReservesDeps["locate"] = async () => ({ where: "custody" as const });
  const run = async (acc: Record<string, SolAccount | null>, locate: BooksReservesDeps["locate"] = inCustody) =>
    (await readBooksReserves({ reader: reader(acc), canister: canister(v, vaults), locate, nowS: 1_700_000_000 })).collections[0];
  const supply = await run(base({ [WX]: { owner: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", lamports: 1n, data: mintBytes(expected + 1n) } }));
  assert.equal(supply.status, "red");
  assert.match(supply.reasons[0], /wX supply .* is not the books' expected supply/);
  const short = await readBooksReserves({ reader: reader(base({ [MODULE]: { owner: "11111111111111111111111111111111", lamports: 890_880n + v.owed - 1n, data: new Uint8Array() } })),
    canister: canister(v, vaults), locate: inCustody, nowS: 1_700_000_000 });
  assert.equal(backingStatus(short.backing, 1_700_000_000).status, "red");
  const elsewhere = await run(base({}), async () => ({ where: "elsewhere" as const, note: "owned by someone else" }));
  assert.equal(elsewhere.status, "red");
  assert.ok(elsewhere.reasons.some((x) => /is not in custody/.test(x)), elsewhere.reasons.join("\n"));
  const vault = await run(base({ [VAULT]: { owner: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", lamports: 1n, data: tokenBytes(1n) } }));
  assert.equal(vault.status, "red");
  assert.match(vault.reasons[0], /vault .* holds 1 wX base units, its book says 2500000000/);
  const unread = await run(base({ [WX]: null }));
  assert.equal(unread.status, "red", "an unreadable mint cannot be green");
  assert.match(unread.reasons[0], /could not be read/);
  const unverified = await run(base({}), async () => ({ where: "unverified" as const, note: "one source timed out" }));
  assert.equal(unverified.status, "unverified");
});

test("the reconciler's configuration: ROOK_HOME is books by default and needs the canister and two Solana providers; the 4663 stack only on the EVM home; the programs home is refused by name", () => {
  const e = (env: Record<string, string>) => { try { parseReconcileConfig(env); return ""; } catch (x) { return (x as Error).message; } };
  assert.match(e({}), /SOL_RPC_URLS is not set/);
  assert.match(e({}), /CANISTER_ID is not set/);
  assert.doesNotMatch(e({}), /RECONCILE_STACK/);
  assert.doesNotMatch(e({}), /ROOK_CORE_PROGRAM/);
  const env = { SOL_RPC_URLS: "https://a.example/rpc,https://b.example/rpc", CANISTER_ID: "rrkah-fqaaa-aaaaa-aaaaq-cai" };
  const cfg = parseReconcileConfig(env);
  assert.equal(cfg.home, "books"); assert.equal(cfg.chainId, SOLANA_CHAIN_ID); assert.equal(cfg.ethRpcUrls!.length, 2);
  assert.match(cfg.launchFile!, /canister-only\/pins\.json$/);
  assert.equal(parseReconcileConfig({ ...env, ROOK_HOME: "books", ROOK_CORE_PROGRAM: "GepazuDZfS6zjwQ3U2zHCgk93xpRkp34AkmEFZKrAd7T" }).home, "books", "a stale ROOK_CORE_PROGRAM is not read");
  assert.deepEqual(parseReconcileConfig({ ...env, RECONCILE_ETH_RPC_URLS: "" }).ethRpcUrls, []);
  assert.match(e({ ...env, ROOK_HOME: "solana" }), /neither books nor robinhood \(the programs home is retired/);
  assert.match(e({ ROOK_HOME: "robinhood" }), /RECONCILE_STACK is not set/);
  assert.equal(parseReconcileConfig({ ROOK_HOME: "robinhood", RECONCILE_STACK: "s.json", RECONCILE_CHAIN_ID: "4663" }).chainId, 4663);
});

test("the locators: an inscription at the listing's custody address (two providers' answer), an Ethereum NFT's owner from every RPC agreeing", async () => {
  const CUSTODY = "bc1p49upe77txy03ysp4m3r4m992tf27d2xgce7yk6w53qg5jtc85exqtkpg3t";
  const L: ListingV = { id: 11, kind: { Inscriptions: { membershipRoot: `0x${"00".repeat(32)}` } }, custody: CUSTODY, custodyEvm: "" };
  const item: ItemV = { itemId: 0, key: Uint8Array.from(Buffer.alloc(32, 1)), inscriptionId: `${"0".repeat(64)}i0` };
  const ord = (address: string) => (async () => new Response(JSON.stringify({ address, satpoint: "x:0:0" }))) as unknown as typeof fetch;
  assert.equal((await locator({ ordFetch: ord(CUSTODY), ethRpcs: [] })(L, item)).where, "custody");
  assert.equal((await locator({ ordFetch: ord("bc1qelsewhere"), ethRpcs: [] })(L, item)).where, "elsewhere");
  assert.equal((await locator({ ordFetch: ord(CUSTODY), ethRpcs: [] })(L, { ...item, inscriptionId: "" })).where, "unverified", "no inscription id");
  const E: ListingV = { id: 0, kind: { EthNft: { chainId: 1n, collection: "0x5af0d9827e0c53e4799bb226655a1de152a425a5" } }, custody: "", custodyEvm: "0x9db3fb117ecfc8fc14ca263e401a4c57a6913d81" };
  const tok = { ...item, key: Uint8Array.from(Buffer.alloc(32, 0).fill(7, 31)) };
  // audit C3-09: every provider is asked its eth_chainId (Ethereum's 0x1 here)
  const chainId = (init: RequestInit | undefined) => JSON.parse(String(init?.body ?? "{}")).method === "eth_chainId";
  const owner = (o: string) => (async (_u: string, init?: RequestInit) => new Response(JSON.stringify({ jsonrpc: "2.0", id: 1,
    result: chainId(init) ? "0x1" : `0x${"0".repeat(24)}${o}` }))) as unknown as typeof fetch;
  assert.equal((await locator({ ordFetch: ord(CUSTODY), ethRpcs: ["https://a", "https://b"], fetch: owner("9db3fb117ecfc8fc14ca263e401a4c57a6913d81") })(E, tok)).where, "custody");
  assert.equal((await locator({ ordFetch: ord(CUSTODY), ethRpcs: ["https://a"], fetch: owner("1111111111111111111111111111111111111111") })(E, tok)).where, "elsewhere");
  assert.equal((await locator({ ordFetch: ord(CUSTODY), ethRpcs: [] })(E, tok)).where, "unverified");
  let n = 0;
  const split = (async (_u: string, init?: RequestInit) => new Response(JSON.stringify({ result: chainId(init) ? "0x1"
    : `0x${"0".repeat(24)}${n++ ? "1".repeat(40) : "9db3fb117ecfc8fc14ca263e401a4c57a6913d81"}` }))) as unknown as typeof fetch;
  assert.equal((await locator({ ordFetch: ord(CUSTODY), ethRpcs: ["https://a", "https://b"], fetch: split })(E, tok)).where, "unverified");
});

/** A classic SPL token account's 165 bytes: mint, owner, amount, a delegate or not, frozen (state 2) or initialized (1). */
function splAccount(mint: string, owner: string, amount: bigint, o: { delegate?: boolean; frozen?: boolean } = {}): Uint8Array {
  const b = Buffer.alloc(165);
  Buffer.from(key58(mint)).copy(b, 0); Buffer.from(key58(owner)).copy(b, 32);
  b.writeBigUInt64LE(amount, 64);
  if (o.delegate) { b.writeUInt32LE(1, 72); b.fill(9, 76, 108); }
  b[108] = o.frozen ? 2 : 1;
  return Uint8Array.from(b);
}

test("a Solana NFT (SolNft listing) is located on Solana: the custodian's token account for its mint holds 1 (frozen fine for a pNFT); green, red, unverified", async () => {
  const TOKEN = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
  const CUSTODIAN = b58(Uint8Array.from({ length: 32 }, () => 0x61)), COLLECTION = b58(Uint8Array.from({ length: 32 }, () => 0x62));
  const v0 = decodeBooksView(FIX)[0]!;
  // the fixture's books as a SolNft listing: its two held items keyed by their mints (the books' key of a Solana NFT is its mint)
  const mints = [0x71, 0x72].map((x) => Uint8Array.from({ length: 32 }, () => x));
  const v: BooksViewV = { ...v0, kind: { SolNft: { collection: key58(COLLECTION), custodian: key58(CUSTODIAN) } },
    items: v0.items.map((it, i) => ({ ...it, key: Buffer.from(mints[i]).toString("hex") })) };
  const L = listingOf(v);
  assert.deepEqual(L, { id: 0, kind: { SolNft: { collection: COLLECTION } }, custody: CUSTODIAN, custodyEvm: "" });
  const expected = v.expected_supply[0]!;
  const base: Record<string, SolAccount | null> = {
    [WX]: { owner: TOKEN, lamports: 1n, data: mintBytes(expected) },
    [MODULE]: { owner: "11111111111111111111111111111111", lamports: 890_880n + v.owed, data: new Uint8Array() },
  };
  const held = (over: (m: string) => SolAccount | null) => Object.fromEntries(mints.map((m) => [ata(CUSTODIAN, b58(m)), over(b58(m))]));
  const run = async (acc: Record<string, SolAccount | null>, solReader: SolReader | null = reader(acc)) =>
    (await readBooksReserves({ reader: reader(acc), canister: canister(v, []), locate: locator({ ordFetch: fetch, ethRpcs: [], solReader }), nowS: 1_700_000_000 })).collections[0];
  // both mints in the custodian's token accounts, amount 1, frozen (a pNFT's always is): green, the kind SolNft
  const green = await run({ ...base, ...held((m) => ({ owner: TOKEN, lamports: 2_039_280n, data: splAccount(m, CUSTODIAN, 1n, { frozen: true }) })) });
  assert.deepEqual([green.status, green.kind, green.reasons], ["green", "SolNft", []], green.reasons.join("\n"));
  assert.equal(green.equation.itemsInCustody, 2);
  // one account empty (the NFT left): RED, the item elsewhere
  const gone = await run({ ...base, ...held((m) => ({ owner: TOKEN, lamports: 1n, data: splAccount(m, CUSTODIAN, m === b58(mints[0]) ? 0n : 1n) })) });
  assert.equal(gone.status, "red");
  assert.ok(gone.reasons.some((r) => /is not in custody: the custodian's token account .* holds 0/.test(r)), gone.reasons.join("\n"));
  // no token account at all: RED
  const none = await run({ ...base, ...held(() => null) });
  assert.equal(none.status, "red");
  assert.match(none.items[0].note!, /has no token account/);
  // a delegate on the account, or no Solana reader: UNVERIFIED, never green
  const delegated = await run({ ...base, ...held((m) => ({ owner: TOKEN, lamports: 1n, data: splAccount(m, CUSTODIAN, 1n, { delegate: true }) })) });
  assert.equal(delegated.status, "unverified");
  assert.match(delegated.items[0].note!, /delegate/);
  const blind = await run({ ...base }, null);
  assert.equal(blind.status, "unverified");
  assert.match(blind.items[0].note!, /no Solana RPC/);
  // the providers disagreeing on the account: unverified (src/solread.ts refuses a split read)
  const split: SolReader = { ...reader(base), account: async (a) => { if (a === WX || a === MODULE) return base[a] ?? null; throw new Error("the Solana providers disagree on the account"); } };
  const disagree = await run({ ...base }, split);
  assert.equal(disagree.status, "unverified");
  // another owner's account at the address (not the custodian's): unverified, not green
  const other = await run({ ...base, ...held((m) => ({ owner: TOKEN, lamports: 1n, data: splAccount(m, COLLECTION, 1n) })) });
  assert.equal(other.status, "unverified");
  assert.equal(decodeTokenAccount({ owner: TOKEN, data: splAccount(b58(mints[0]), CUSTODIAN, 1n, { frozen: true }) })!.frozen, true);
  // pinRefs reads a SolNft pin's kind
  assert.equal(pinRefs({ collections: [{ index: 2, slug: "mad_lads", name: "Mad Lads", symbol: "wMADLAD", kind: "SolNft { collection: [..], custodian: [..] }" }] }).get(2)!.kind, "SolNft");
});

test("the Solana reader: every provider asked at finalized, an account must be identical at each, a balance the least; only reads", async () => {
  const acc = (data: string, lamports: number) => ({ value: { owner: MODULE, lamports, data: [Buffer.from(data).toString("base64"), "base64"] } });
  const calls: { url: string; body: { method: string; params: unknown[] } }[] = [];
  const mk = (b: string) => (async (url: string, init: RequestInit) => {
    const body = JSON.parse(String(init.body)); calls.push({ url, body });
    return new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, result: acc(url.includes("b.") ? b : "same", url.includes("b.") ? 5 : 9) }));
  }) as unknown as typeof fetch;
  const r = solReader(["https://a.example", "https://b.example"], mk("same"));
  const a = await r.account("x");
  assert.equal(a!.lamports, 5n);
  assert.deepEqual((calls[0].body.params[1] as { commitment: string }).commitment, "finalized");
  await assert.rejects(solReader(["https://a.example", "https://b.example"], mk("other")).account("x"), /disagree/);
});

test("one books run end to end: read, combine with the caps, persist, alert on red, serve a page that names the slot", async () => {
  const v = decodeBooksView(FIX)[0]!;
  const expected = v.expected_supply[0]!;
  const accounts: Record<string, SolAccount> = {
    [WX]: { owner: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", lamports: 1n, data: mintBytes(expected + 9n * 1_000_000_000n) },
    [MODULE]: { owner: "11111111111111111111111111111111", lamports: 890_880n + v.owed, data: new Uint8Array() },
  };
  const cfg = parseReconcileConfig({ SOL_RPC_URLS: "https://a.example", CANISTER_ID: "rrkah-fqaaa-aaaaa-aaaaq-cai", RECONCILE_STATE_DIR: mkdtempSync(join(tmpdir(), "booksrec-")) });
  let lastBacking: Parameters<typeof combine>[4];
  const service = { reserves: async () => {
    const r = await readBooksReserves({ reader: reader(accounts), canister: canister(v, []), locate: async () => ({ where: "custody" as const }), nowS: 1_700_000_000 });
    lastBacking = r.backing; return r.collections;
  } };
  const sent: string[] = [];
  const channel: Channel = { send: async (t) => { sent.push(t); } };
  const s = await runOnce({ cfg, service, caps: async () => ({ kind: "absent", note: "test", stackStage: 0, at: 1_700_000_000 }), channel, store: new Store(cfg.stateDir), log: logger(() => {}),
    nowMs: () => 1_700_000_000 * 1000, stackStage: 0, backing: async () => lastBacking! });
  assert.equal(s.servedStatus, "red");
  assert.match(s.servedReasons.join(), /collection-0: wX supply/);
  assert.equal(sent.length, 1);
  assert.match(page(served(s, 1_700_000_000, 10_800, SOLANA_CHAIN_ID)), /Solana slot \(finalized\) 1234/);
  assert.match(page(s), /The floor modules' SOL \(slot 1234\)/);
});

/** A DLMM PositionV2's bytes as the canister decodes them: the discriminator, the pair, the owner, the bin range at 7912/7916. */
function positionBytes(pair: string, owner: string, lo: number, hi: number): Uint8Array {
  const b = Buffer.alloc(8_120);
  Buffer.from(anchorDisc("account", "PositionV2")).copy(b, 0);
  Buffer.from(key58(pair)).copy(b, 8); Buffer.from(key58(owner)).copy(b, 40);
  b.writeInt32LE(lo, 7_912); b.writeInt32LE(hi, 7_916);
  return Uint8Array.from(b);
}
import { key as key58 } from "../src/fuel/sol.js";

test("W11: the pool desk's books are checked against its position's owner: green when covered, red on a shortfall or a range the books do not say", async () => {
  const v = decodeBooksView(FIX)[0]!;
  const expected = v.expected_supply[0]!;
  const PAIR = b58(Uint8Array.from({ length: 32 }, () => 0x51)), DESK = b58(Uint8Array.from({ length: 32 }, () => 0x52)), POS = b58(Uint8Array.from({ length: 32 }, () => 0x53));
  const desk: DeskViewV = { coll: 0, pair: PAIR, position: POS, lo: 560, hi: 598, active: 579, native: 1_500_000_000n, wsol: 2_000_000_000n, wx: 3_000_000_000n, read_at: 1n,
    fee_lamports: 0n, swept: 0n, fee_units: 0n, pos_x: 0n, pos_y: 0n, arb_profit: 0n, arbs_last_hour: 0 };
  const base = (over: Record<string, SolAccount | null>): Record<string, SolAccount | null> => ({
    [WX]: { owner: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", lamports: 1n, data: mintBytes(expected) },
    [MODULE]: { owner: "11111111111111111111111111111111", lamports: 890_880n + v.owed, data: new Uint8Array() },
    [POS]: { owner: DLMM_PROGRAM, lamports: 1n, data: positionBytes(PAIR, DESK, 560, 598) },
    [DESK]: { owner: "11111111111111111111111111111111", lamports: 1_600_000_000n, data: new Uint8Array() },
    [ata(DESK, NATIVE_MINT)]: { owner: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", lamports: 1n, data: tokenBytes(2_000_000_000n) },
    [ata(DESK, WX)]: { owner: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", lamports: 1n, data: tokenBytes(3_500_000_000n) },
    ...over,
  });
  const inCustody: BooksReservesDeps["locate"] = async () => ({ where: "custody" as const });
  const run = async (acc: Record<string, SolAccount | null>, d = desk) =>
    (await readBooksReserves({ reader: reader(acc), canister: canister(v, [], MODULE, [d]), locate: inCustody, nowS: 1_700_000_000 })).collections[0];
  const green = await run(base({}));
  assert.deepEqual([green.status, green.reasons], ["green", []], green.reasons.join("\n"));
  assert.equal(green.desk?.desk, DESK);
  assert.deepEqual(green.desk?.native, ["1500000000", "1600000000"]);
  const short = await run(base({ [ata(DESK, WX)]: { owner: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", lamports: 1n, data: tokenBytes(2_999_999_999n) } }));
  assert.equal(short.status, "red");
  assert.match(short.reasons[0], /desk 0: the desk's wX account holds 2999999999 base units, its books say 3000000000/);
  const range = await run(base({}), { ...desk, hi: 600 });
  assert.equal(range.status, "red");
  assert.match(range.reasons[0], /range is \[560, 598\], the books say \[560, 600\]/);
  const notPos = await run(base({ [POS]: { owner: DLMM_PROGRAM, lamports: 1n, data: new Uint8Array(100) } }));
  assert.equal(notPos.status, "red");
  assert.match(notPos.reasons[0], /is not a DLMM position/);
  const none = await run(base({}), { ...desk, pair: "" });
  assert.equal(none.status, "green", "no desk opened yet: nothing to check");
  assert.equal(none.desk, null);
});

test("2026-10-12: a Robinhood Chain NFT's owner is read on 4663 (its own RPCs), never on Ethereum's; none configured: unverified", async () => {
  const CUSTODY_EVM = "9db3fb117ecfc8fc14ca263e401a4c57a6913d81";
  const RH: ListingV = { id: 2, kind: { EthNft: { chainId: 4663n, collection: "0xe3b34c4bb0f12c82143745eee6a6cf4e3154b1fa" } }, custody: "", custodyEvm: `0x${CUSTODY_EVM}` };
  const tok: ItemV = { itemId: 0, key: Uint8Array.from(Buffer.alloc(32, 0).fill(7, 31)), inscriptionId: "" };
  const asked: string[] = [];
  // audit C3-09: each provider says which chain it serves (`eth_chainId`); `serves` maps a URL to its chain (4663 unless named)
  const owner = (o: string, serves: Record<string, number> = {}, down: string[] = []) => (async (u: string, init?: RequestInit) => {
    asked.push(String(u));
    if (down.includes(String(u))) return new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, error: { code: 429, message: "rate limited" } }));
    const isChainId = JSON.parse(String(init?.body ?? "{}")).method === "eth_chainId";
    return new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, result: isChainId ? `0x${(serves[String(u)] ?? 4663).toString(16)}` : `0x${"0".repeat(24)}${o}` }));
  }) as unknown as typeof fetch;
  const ord = (async () => new Response("{}")) as unknown as typeof fetch;
  const loc = (f: typeof fetch, by?: Record<string, string[]>) => locator({ ordFetch: ord, ethRpcs: ["https://eth-a", "https://eth-b"], rpcsByChain: by, fetch: f });
  assert.equal((await loc(owner(CUSTODY_EVM), { "4663": ["https://rh-a", "https://rh-b"] })(RH, tok)).where, "custody");
  assert.deepEqual([...new Set(asked)].sort(), ["https://rh-a", "https://rh-b"], "4663's RPCs only");
  assert.equal((await loc(owner("1".repeat(40)), { "4663": ["https://rh-a", "https://rh-b"] })(RH, tok)).where, "elsewhere");
  // fixed C3-09: one 4663 provider configured, one failing, or one serving another chain: unverified, never the other's word alone
  const one = await loc(owner("1".repeat(40)), { "4663": ["https://rh-a"] })(RH, tok);
  assert.equal(one.where, "unverified");
  assert.match(String(one.note), /two RPCs at least.*C3-09/);
  const failing = await loc(owner("1".repeat(40), {}, ["https://rh-b"]), { "4663": ["https://rh-a", "https://rh-b"] })(RH, tok);
  assert.equal(failing.where, "unverified");
  assert.match(String(failing.note), /1 of the 2 RPCs of chain 4663 did not answer/);
  const wrong = await loc(owner(CUSTODY_EVM, { "https://eth-a": 1 }), { "4663": ["https://rh-a", "https://eth-a"] })(RH, tok);
  assert.equal(wrong.where, "unverified");
  assert.match(String(wrong.note), /serves chain 1, not 4663/);
  const none = await loc(owner(CUSTODY_EVM))(RH, tok);
  assert.equal(none.where, "unverified");
  assert.match(String(none.note), /chain 4663.*RECONCILE_RH_RPC_URLS/);
  assert.deepEqual(rpcsOf({ ethRpcs: ["https://e"] }, 1n), ["https://e"]);
  assert.deepEqual(rpcsOf({ ethRpcs: ["https://e"], rpcsByChain: { "4663": ["https://r"] } }, 4663n), ["https://r"]);
  // the reconciler's configuration: Robinhood Chain's keyless pair by default, its own list, or off
  const base = { SOL_RPC_URLS: "https://a.example/rpc,https://b.example/rpc", CANISTER_ID: "rrkah-fqaaa-aaaaa-aaaaq-cai" };
  assert.deepEqual(parseReconcileConfig(base).rhRpcUrls, RH_RPC_KEYLESS);
  assert.deepEqual(parseReconcileConfig({ ...base, RECONCILE_RH_RPC_URLS: "https://rh.example" }).rhRpcUrls, ["https://rh.example"]);
  assert.deepEqual(parseReconcileConfig({ ...base, RECONCILE_RH_RPC_URLS: "" }).rhRpcUrls, []);
});
