// The web build's context is this directory alone (it is what gets deployed), so the documents it renders are copied in and committed.
// Run from the repository it copies; in a Docker build where ../docs does not exist it does nothing and the committed copies are used.
import { copyFileSync, existsSync, mkdirSync, readdirSync } from "node:fs";
const here = new URL("../", import.meta.url);
const docs = new URL("../../docs/", import.meta.url);
mkdirSync(new URL("src/content/", here), { recursive: true });
if (existsSync(docs)) copyFileSync(new URL("trust-model.md", docs), new URL("src/content/trust-model.md", here));
// a deployment for a public chain is put in the app by hand, on purpose: copy deployments/<chainid>.json here and rebuild.
const deployments = new URL("../../deployments/", import.meta.url);
mkdirSync(new URL("src/deployments/", here), { recursive: true });
if (existsSync(deployments)) for (const f of readdirSync(deployments)) // ours are never bundled: 31337 is a developer's machine, the staging fork (whatever staging/chain-id.json says) is read from its own /info at run time. A labelled test IS
// bundled, under its own name, and the app reads it only behind ?net=test (docs/phase6.md).
if (f === "4663.json" || f === "test-4663.json")   // Robinhood Chain and nothing else: no list of our own ids to keep in step
  copyFileSync(new URL(f, deployments), new URL(`src/deployments/${f}`, here));
