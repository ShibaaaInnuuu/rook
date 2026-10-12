import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import { netOf, pick } from "../src/lib/deployments";
import { STAGING_ID, STAGING_NAME, STAGING_CURRENCY, robinhood } from "../src/lib/chains";

const SOURCE = JSON.parse(readFileSync(new URL("../../staging/chain-id.json", import.meta.url), "utf8")) as { chainId: number; name: string; currency: { name: string; symbol: string; decimals: number }; never: number[] };

test("docs/phase6.md: a deployment file labelled test is never read as THE deployment, and the default site stays 'nothing is live'", () => {
  const t = { chainId: 4663, label: "test", registry: "0xT" }, m = { chainId: 4663, label: "mainnet", registry: "0xM" }, s = { chainId: STAGING_ID, label: "staging", registry: "0xS" };
  assert.deepEqual(pick([t], 4663), { live: null, test: t }, "only a test exists: the site has no deployment");
  assert.deepEqual(pick([t, m, s], 4663), { live: m, test: t });
  assert.deepEqual(pick([s], 4663), { live: null, test: null }, "a staging file is not Robinhood Chain's");
  assert.equal(pick([{ chainId: 4663 }], 4663).live?.chainId, 4663, "a file from before labels existed is the deployment");
  assert.deepEqual(["?net=staging", "?net=test", "?net=mainnet", "?net=", "", "?demo=sample"].map(netOf), ["staging", "test", null, null, null, null]);
});

test("the staging trade box and the faucet exist only behind the staging net, and the legal pages leave entity and law as marked blanks", () => {
  // The ticket is drawn on every network so a reader can see what trading involves, but it can only ever SIGN on staging:
  // off it, the reason replaces the action rather than the box disappearing. The guarantee is that "staging" gates the send.
  const trade = readFileSync(new URL("../src/components/Terminal.tsx", import.meta.url), "utf8");
  assert.ok(/const staging = env\?\.net === "staging";/.test(trade), "the terminal knows which net it is on");
  assert.ok(/!staging \? "This is not staging\./.test(trade), "off staging the reason is stated, not hidden");
  assert.ok(/disabledWhy=\{why/.test(trade), "and that reason disables the button that would sign");
  assert.ok(!/StagingTrade/.test(readFileSync(new URL("../src/pages/Coin.tsx", import.meta.url), "utf8")), "the old box is gone");
  const cfg = readFileSync(new URL("../src/config.ts", import.meta.url), "utf8");
  for (const k of ["LEGAL_ENTITY", "GOVERNING_LAW", "LEGAL_CONTACT", "X_HANDLE"]) assert.ok(new RegExp(`export const ${k} = "";`).test(cfg), `${k} is left blank for the owner`);
  const legal = readFileSync(new URL("../src/pages/Legal.tsx", import.meta.url), "utf8");
  assert.ok(/not legal advice/.test(legal) && /reviewed by one before any contract is deployed/.test(legal));
  assert.ok(!/guarantee[sd]? (a |an )?(return|profit)|price will|floor (will|goes) up/i.test(legal), "no price or return claims");
});

test("the staging chain id is the repository's one number (staging/chain-id.json), is no real network's, and its money cannot be mistaken for ETH", () => {
  assert.equal(STAGING_ID, SOURCE.chainId);
  assert.ok(!SOURCE.never.includes(STAGING_ID) && SOURCE.never.includes(robinhood.id) && SOURCE.never.includes(46630), "never Robinhood Chain, never its public testnet");
  assert.deepEqual([STAGING_NAME, STAGING_CURRENCY], [SOURCE.name, SOURCE.currency]);
  assert.notEqual(STAGING_CURRENCY.symbol, "ETH");
});
