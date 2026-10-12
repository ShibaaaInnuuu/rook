// W15: the canister-only rehearsal's tooling, offline (docs/v9/rehearsal-sol-devnet.md §8). No replica, no devnet, no IC: the install
// argument decoded with the did's Init, the argument parser and the phase list of run.sh (a dry run that prints every command and runs
// none), the checklist's parsers (books.py) over the canister's own saved view bytes (tests/fixtures/views/*.hex, decoded the way the
// monitor decodes them), books.py compiled. Needs the monitor's dependencies (cd monitor && npm ci).
//   node --test ops/rehearsal/sol-devnet/test/*.test.mjs
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

const HERE = new URL("../", import.meta.url).pathname;
const ROOT = new URL("../../../../", import.meta.url).pathname;
const DID = readFileSync(ROOT + "autobuyer/canister-rs/etch_autobuyer.did", "utf8");
const URLS = ["https://api.devnet.solana.com", "https://devnet.rpcpool.com"];
const sh = (args, env = {}) => spawnSync("bash", [HERE + "run.sh", ...args], { encoding: "utf8", env: { ...process.env, ...env } });
const py = (args) => spawnSync("python3", ["-I", HERE + "books.py", ...args], { encoding: "utf8" });

/** The field names of `type <name> = record { … }` in the did (comments dropped), or of a nested record found by a regex over the text. */
function didFields(text) {
  return [...text.replace(/\/\/[^\n]*/g, "").matchAll(/^\s*([a-z_]+)\s*:/gm)].map((m) => m[1]).sort();
}
const didType = (name) => DID.match(new RegExp(`^type ${name} = record \\{\\n([\\s\\S]*?)^\\};`, "m"))[1];

test("W15: the books install argument decodes with the did's Init and says rook_core = books, every Ethereum collection on the books", async () => {
  const { booksInit, encodeChecked, LAUNCH, initOfArgv } = await import("../canister-init.mjs");
  const { decodeInit, Init, Home } = await import(ROOT + "monitor/src/candid.mjs");
  // the monitor's Init is the did's: same top-level fields, same Solana home fields (the Init may gain fields; this catches it)
  assert.deepEqual(Object.keys(Init._fields ? Object.fromEntries(Init._fields) : {}).sort(), didFields(didType("Init")));
  const solHome = DID.match(/^type Home = variant \{\n\s*Solana : record \{\n([\s\S]*?)^\s*\};/m)[1];
  const homeFields = Home._fields.find(([k]) => k === "Solana")[1]._fields.map(([k]) => k).sort();
  assert.deepEqual(homeFields, didFields(solHome));
  assert.deepEqual(didFields(didType("EthCollection")), ["chain_id", "collection", "max_premium_bps", "max_purchase_wei", "module", "nft_value_bps", "token_bound", "wrapper"]);
  // the first five of the registry (BAYC, Milady, Mad Lads, SMB Gen2, Bitcoin Frogs): the argument from the command line, decoded back
  const hex = encodeChecked(initOfArgv(["--books", "test_key_1", URLS.join(","), "signet", "rook-w15-test", "--count", "5"]));
  const d = decodeInit(hex);
  assert.ok(d.ok, d.error);
  const home = d.init.home[0].Solana;
  assert.equal(home.rook_core, "books");
  assert.equal(home.cluster, "devnet");
  assert.deepEqual(home.rpc_urls, URLS);
  assert.equal(home.nonce_seed, "rook-w15-test");
  assert.deepEqual(home.rook_backing, []);
  assert.deepEqual(home.drand_urls, []);
  assert.deepEqual(d.init.wrapper, [], "no frog wrapper on the books home (runbook §1)");
  assert.deepEqual(d.init.frog_funding, []);
  assert.equal(d.init.key_name, "test_key_1");
  assert.equal(d.init.accept_production_key_name, false);
  assert.equal(d.init.btc_network, "signet");
  const cols = d.init.eth_nft[0].collections;
  assert.equal(cols.length, 2, "the first five's two Ethereum collections (the frogs and the Solana collections need nothing in the Init: their pins carry their custody)");
  for (const c of cols) assert.deepEqual([c.wrapper, c.module], ["books", "books"], c.collection);
  const pilot = JSON.parse(readFileSync(ROOT + "deployments/launch/nft-pilot.json", "utf8"));
  const addr = Object.fromEntries(pilot.collections.map((c) => [c.slug, c.collection.toLowerCase()]));
  assert.deepEqual(cols.map((c) => c.collection.toLowerCase()), ["bayc", "milady"].map((s) => addr[s]));
  assert.equal(d.init.eth_nft[0].chain_id, "1");
  assert.equal(d.init.eth_nft[0].deposits, false);
  assert.equal(d.init.evm_chains[0][0].chain_id, "1", "Ethereum mainnet, read only");
  // --count 2: bayc and milady only; the default count is all 38 the ceremony pins (the 11 Ethereum collections in the Init)
  const two = decodeInit(encodeChecked(booksInit({ sol: URLS, count: 2 }))).init;
  assert.deepEqual(two.eth_nft[0].collections.map((c) => c.collection.toLowerCase()), [addr.bayc, addr.milady]);
  assert.equal(decodeInit(encodeChecked(booksInit({ sol: URLS }))).init.eth_nft[0].collections.length, 11);
  // the pinned set is the canister's registry: ORDER's first LAUNCH_COUNT (operator decision 2026-10-12: every collection with a pipeline)
  const order = readFileSync(ROOT + "autobuyer/canister-rs/src/coins/unlock.rs", "utf8");
  const all = [...order.matchAll(/^\s*e\("([^"]+)", "[^"]+", "[^"]+", (?:Eth|Btc|Sol|Ord)\),/gm)].map((m) => m[1]);
  assert.match(order, /pub const LAUNCH_COUNT: usize = 38;/);
  assert.deepEqual(all.slice(0, 38), LAUNCH);
});

test("W15: the install argument's own refusals", async () => {
  const { booksInit } = await import("../canister-init.mjs");
  assert.throws(() => booksInit({ key: "key_1", sol: URLS }), /key_1 is the production key's name/);
  assert.throws(() => booksInit({ sol: ["https://api.mainnet-beta.solana.com", URLS[1]] }), /mainnet/);
  assert.throws(() => booksInit({ sol: [URLS[0], URLS[0]] }), /two distinct/);
  assert.throws(() => booksInit({ sol: URLS, btc: "mainnet" }), /signet or testnet4/);
  assert.throws(() => booksInit({ sol: URLS, count: 39 }), /--count takes 1 to 38/);
  assert.throws(() => booksInit({ sol: URLS, count: 0 }), /--count takes 1 to 38/);
  assert.ok(booksInit({ sol: URLS, btc: "testnet4", count: 1 }).eth_nft[0].collections.length === 1);
  // the script itself refuses key_1 outside a local replica, and takes it on one
  const r = spawnSync("node", [HERE + "canister-init.mjs", "--books", "key_1", URLS.join(",")], { encoding: "utf8", env: { ...process.env, ROOK_LOCAL_REPLICA: "" } });
  assert.notEqual(r.status, 0);
  const ok = spawnSync("node", [HERE + "canister-init.mjs", "--books", "key_1", URLS.join(","), "signet", "s", "--count", "2"], { encoding: "utf8", env: { ...process.env, ROOK_LOCAL_REPLICA: "1" } });
  assert.equal(ok.status, 0, ok.stderr);
  assert.match(ok.stdout.trim(), /^[0-9a-f]+$/);
});

test("W15: run.sh --books: the argument parser, the phase list and a dry run that prints every command and runs none", () => {
  const job = mkdtempSync(join(tmpdir(), "rook-w15-test-"));
  try {
    const r = sh(["--books", "--local-replica", "--no-build", "--dry-run", "--job", job]);
    assert.equal(r.status, 0, r.stdout + r.stderr);
    const out = r.stdout;
    assert.match(out, /phases: build,canister,pins,fund,open,checklist,cleanup/);
    assert.match(out, /DRY RUN: every command printed, none run/);
    assert.match(out, /local \(key_1\)/);
    assert.match(out, /the float K_main needs: 0\.1 SOL/);
    for (const want of ["canister-init.mjs --books key_1", "canister create rooksol --network local", "chunked-install.sh' --network local --mode install",
      "books.py addresses", "cargo run -q --example launch_pins -- --evm-custody", "--count 2", "books.py pins", "books.py checklist",
      "--chunked", "--local --host http://127.0.0.1:4957", "canister delete rooksol --network local -y"]) {
      assert.ok(out.includes(want), `missing: ${want}\n${out}`);
    }
    assert.match(out, /fund: skipped on the local replica without --payer/);
    assert.match(out, /== PASS/);
    // nothing ran: no job file but the directory itself
    assert.ok(!out.includes("FAIL"));
    // the devnet form, with every switch
    const payer = join(job, "payer.json");
    spawnSync("bash", ["-c", `echo '[]' > '${payer}'`]);
    const d = sh(["--books", "--devnet", "--payer", payer, "--next-to", "aaaaa-aa", "--count", "5", "--intent", "--yes", "--dry-run", "--job", job]);
    assert.equal(d.status, 0, d.stdout + d.stderr);
    for (const want of ["ic (test_key_1)", "the float K_main needs: 0.17 SOL + the intent: 0.0515 SOL from the payer", "canister create rooksol --network ic --with-cycles 3000000000000 --next-to aaaaa-aa",
      "--network ic --mode install", "--count 5", "--sol-custodian <sol>", "books.py fund", "--payer", "books.py open", "--intent", "canister delete rooksol --network ic -y (the trap"]) {
      assert.ok(d.stdout.includes(want), `missing: ${want}\n${d.stdout}`);
    }
    assert.match(d.stdout, /dry-run: cd '.*autobuyer\/canister-rs' && cargo build --locked --release --target wasm32-unknown-unknown/, "the build is printed, not run");
    // a subset of the phases
    const p = sh(["--books", "--local-replica", "--no-build", "--dry-run", "--job", job, "--phases", "pins,open"]);
    assert.equal(p.status, 0, p.stderr);
    assert.match(p.stdout, /phases: pins,open/);
    assert.ok(!p.stdout.includes("canister create"));
  } finally {
    rmSync(job, { recursive: true, force: true });
  }
  // the refusals, every one before anything is created
  const refused = (args, why) => { const r = sh(args); assert.equal(r.status, 2, args.join(" ")); assert.match(r.stdout + r.stderr, why, args.join(" ")); };
  refused(["--books", "--devnet", "--dry-run"], /needs --payer/);
  refused(["--books", "--local-replica", "--dry-run", "--rpc-urls", "https://api.mainnet-beta.solana.com,https://b"], /names mainnet/);
  refused(["--books", "--local-replica", "--dry-run", "--phases", "build,bogus"], /knows the phases/);
  refused(["--books", "--local-validator", "--dry-run"], /--books is the canister-only rehearsal/);
  refused(["--books", "--local-replica", "--dry-run", "--intent"], /--intent sends the intent's deposit from the payer/);
  refused(["--books", "--local-replica", "--dry-run", "--count", "39"], /--count takes 1 to 38/);
  refused(["--books", "--local-replica", "--dry-run", "--job", ROOT + "ops/rehearsal/sol-devnet/job"], /inside the repository/);
  refused(["--books", "--local-replica", "--dry-run", "--evidence", "/nonexistent/evidence.json"], /not a file/);
  refused(["--books", "--local-replica", "--dry-run", "--bogus"], /unknown argument/);
});

test("W15: books.py compiles, and its parsers read the canister's saved views the way the monitor decodes them", () => {
  const c = spawnSync("python3", ["-m", "py_compile", HERE + "books.py"], { encoding: "utf8" });
  assert.equal(c.status, 0, c.stderr);
  const r = py(["parse", ROOT + "autobuyer/canister-rs/tests/fixtures/views"]);
  assert.equal(r.status, 0, r.stderr);
  const v = JSON.parse(r.stdout);
  // books_view: the fixture's collection 0 with two items, one ticket, a fresh floor: every consistency rule holds
  assert.equal(v.books_view.coll, 0);
  assert.equal(v.books_view.fresh_floor, true);
  assert.ok(v.books_view.rules.every((x) => x[1] === true), JSON.stringify(v.books_view.rules));
  assert.deepEqual(v.books_view.rules.map((x) => x[0]), ["owed = backing + reserve + surplus + revenue", "expected_supply = (held - open tickets) * 1 wX + sol_backed", "not halted", "items listed = held"]);
  const wrong = v.books_view.rules_wrong_mint.find((x) => x[0] === "wx_mint = the pinned mint");
  assert.equal(wrong[1], false, "a pinned mint that is not the books' is caught");
  // coins_log: the config wait, the mint landed
  assert.deepEqual(v.coins_log.waits, [["open_config:1", "the config's rent is not covered", 1700000000, 1]]);
  assert.deepEqual(v.coins_log.landed, [["open_mint", 1700000500, "collection 0's mint"]]);
  // the opening queue (coins_request_open, oldest first) and a listing's kind: a SolNft view has no EVM or BTC custody to check
  assert.deepEqual(v.coins_log.queue, [[2, 1700000600]]);
  assert.equal(v.kind, "EthNft");
  assert.equal(v.kind_solnft, "SolNft");
  // desk_status: the fixture's halted collection is what the checklist flags; treasury_status: its waits and the drand disagreement
  assert.equal(v.desk_status.find((x) => x[0] === "no collection halted")[1], false);
  assert.equal(v.desk_status.find((x) => x[0] === "no unexplained memo")[1], true);
  assert.equal(v.treasury_status.find((x) => x[0] === "no treasury wait")[1], false);
  assert.equal(v.treasury_status.find((x) => x[0] === "drand relays agree")[1], false);
  assert.equal(v.treasury_status.find((x) => x[0] === "cranks on")[1], true);
  assert.equal(v.pool_desks, 1);
  // the float: the reserve the money flows keep + each opening (0.0226 SOL) + the nonce account, rounded up to 0.01 SOL
  assert.deepEqual(v.float, { 2: 100000000, 5: 170000000 });
  // the monitor's own decoder is what views-decode.mjs uses (books_view through candid.mjs's REPLY_TYPES)
  const dec = spawnSync("node", [HERE + "views-decode.mjs", "books_view", ROOT + "autobuyer/canister-rs/tests/fixtures/views/books-view.hex"], { encoding: "utf8" });
  assert.equal(dec.status, 0, dec.stderr);
  assert.equal(JSON.parse(dec.stdout)[0].expected_supply[0], "1009985022");
  const bad = spawnSync("node", [HERE + "views-decode.mjs", "nope", "x"], { encoding: "utf8" });
  assert.notEqual(bad.status, 0);
  // the driver refuses a mainnet RPC before anything else
  const m = py(["checklist", "dfx", "ic", "/nonexistent", "--rpc", "https://api.mainnet-beta.solana.com"]);
  assert.equal(m.status, 2);
  assert.match(m.stdout, /refused: the RPC names mainnet/);
});
