// The verifier is on the path that signs. Needs the local replica; skips otherwise.
//
// docs/security/audit-btc-v1.md B3: the digest used to be computed in JavaScript, on the operator's machine, from a
// transaction the canister never saw, while psbt.rs's checks sat with no non-test caller. This drives the whole path —
// the canister's own runestone encoder, a transaction built by a library it has never seen, its own verify, its own
// BIP341 digest, its own tweak, and a real threshold signature — and asserts the pieces agree.
import assert from "node:assert/strict";
import { test } from "node:test";
import { execFileSync } from "node:child_process";

const HERE = new URL("../", import.meta.url).pathname;
let up = true;
try {
  execFileSync(`${process.env.HOME}/Library/Application Support/org.dfinity.dfx/bin/dfx`,
    ["canister", "call", "autobuyer", "paused"], { cwd: `${HERE}canister-rs`, stdio: "pipe" });
} catch { up = false; }

test("the canister signs the transaction it verified, and nothing else", { skip: !up && "no local replica" }, () => {
  const out = execFileSync("node", ["sighash-e2e.mjs"], { cwd: HERE, encoding: "utf8" });
  assert.match(out, /the canister's digest equals an independent library's/, out.slice(-400));
  assert.match(out, /verifies under the OUTPUT key:   true/);
  assert.match(out, /and NOT under the internal one:  true/);
});
