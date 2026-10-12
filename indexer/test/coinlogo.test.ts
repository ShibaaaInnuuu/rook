import assert from "node:assert/strict";
import { test } from "node:test";
import { encodeFunctionData, type Abi } from "viem";
import { LAUNCHER_CALLS } from "../src/abis.js";
import { openDb } from "../src/db.js";
import { Follower } from "../src/follow.js";
import { Views } from "../src/views.js";
import { Live } from "../src/live.js";
import { clockOf, loadTape, replay } from "./helpers.js";

/** A real `Launcher.launch` call carrying one logo, so the follower has something true to decode. */
const launchCall = (logo: string) => encodeFunctionData({
  abi: LAUNCHER_CALLS as Abi,
  functionName: "launch",
  args: [{
    name: "Local Stone", symbol: "STONE", logo, description: "",
    socials: { twitter: "", telegram: "", discord: "", website: "", farcaster: "" },
    creatorTaxBps: 200, expectedEconomics: `0x${"0".repeat(64)}`, salt: `0x${"0".repeat(64)}`,
    launchConfigId: 0n, listingId: 0n,
  }],
});

const logosAfterPass = async (input: string | null): Promise<(string | null)[]> => {
  const t = loadTape(), db = openDb(":memory:");
  const rpc = replay(t, (method) => (method === "eth_getTransactionByHash"
    ? (input === null ? null : { input: launchCall(input) })          // null: a node that has forgotten the transaction
    : undefined));
  await new Follower({ db, rpc, deployment: t.deployment, confirmations: 0 }).pass();
  const views = new Views(db, new Live(rpc, t.deployment.ponsFactory), () => clockOf(t));
  return views.coins(50, 0).items.map((c) => c.logoUrl);
};

test("the logo is decoded from the launch transaction and served as CoinSummary.logoUrl", async () => {
  const ipfs = "ipfs://bafkreicuspukrnaisnzred73svouxh3favyf4frtdbjlha36juefmx2ygq";
  const logos = await logosAfterPass(ipfs);
  assert.ok(logos.length > 0, "the recording holds at least one coin");
  assert.ok(logos.every((l) => l === ipfs), `every coin wears the logo its launch carried: ${JSON.stringify(logos)}`);
});

test("a hostile logo, an empty one and an unreadable transaction all serve null, so the coin keeps its monogram", async () => {
  for (const hostile of ["javascript:alert(1)", "data:text/html,<script>alert(1)</script>", ""]) {
    assert.ok((await logosAfterPass(hostile)).every((l) => l === null), hostile || "(empty)");
  }
  assert.ok((await logosAfterPass(null)).every((l) => l === null), "no transaction");
});

test("a coin indexed before the logo existed is filled in by the follower's own retry, and asked only once", async () => {
  const t = loadTape(), db = openDb(":memory:");
  const ipfs = "ipfs://bafkreicuspukrnaisnzred73svouxh3favyf4frtdbjlha36juefmx2ygq";
  let txReads = 0;
  const rpc = replay(t, (method) => {
    if (method !== "eth_getTransactionByHash") return undefined;
    txReads++;
    return { input: launchCall(ipfs) };
  });
  const f = new Follower({ db, rpc, deployment: t.deployment, confirmations: 0 });
  await f.pass();
  const coins = db.prepare(`SELECT count(*) AS n FROM coins`).get() as { n: number };
  assert.equal(txReads, coins.n, "one transaction read per coin, at its launch");

  // the state a database written before this phase is in: the column exists, nothing has been asked
  db.exec(`UPDATE coin_meta SET logo = NULL, logo_at = NULL`);
  await f.pass();
  assert.equal(txReads, coins.n * 2, "the retry asks again, once per coin");
  const filled = db.prepare(`SELECT count(*) AS n FROM coin_meta WHERE logo = ?`).get(ipfs) as { n: number };
  assert.equal(filled.n, coins.n, "and fills every one of them");

  await f.pass();
  assert.equal(txReads, coins.n * 2, "a coin already asked is not asked again");
});
