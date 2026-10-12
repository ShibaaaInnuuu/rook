// The web build's context is this directory alone (it is what gets deployed), so anything it takes from docs/ is copied in and
// committed. Run from the repository; in a Docker build where ../docs does not exist it does nothing and the committed copies are used.
//
// The Docs page does NOT come from docs/: it renders the user documentation written for the site, src/content/docs/*.md, edited
// in place (the engineering documents under docs/ stay there). The one document still copied is the trust model, which the Trust
// page renders word for word.
import { copyFileSync, existsSync, mkdirSync, readdirSync } from "node:fs";
const here = new URL("../", import.meta.url);
const docs = new URL("../../docs/", import.meta.url);
/** Copied from docs/ into src/content/, byte for byte: the file under docs/ and the name it is stored under. */
export const DOCS = [
  ["trust-model.md", "trust-model"],
];
mkdirSync(new URL("src/content/", here), { recursive: true });
if (existsSync(docs)) for (const [from, name] of DOCS) copyFileSync(new URL(from, docs), new URL(`src/content/${name}.md`, here));
// a deployment for a public chain is put in the app by hand, on purpose: copy deployments/<chainid>.json here and rebuild.
const deployments = new URL("../../deployments/", import.meta.url);
mkdirSync(new URL("src/deployments/", here), { recursive: true });
if (existsSync(deployments)) for (const f of readdirSync(deployments))
  // Robinhood Chain and nothing else. A labelled test IS bundled, under its own name, and the app reads it only behind ?net=test.
  // The stack file (docs/v6/phase1-scope.md §8.3) is what a Phase-1 build may send to; without it a Phase-1 TxButton refuses.
  if (f === "4663.json" || f === "test-4663.json" || f === "stack-4663.json") copyFileSync(new URL(f, deployments), new URL(`src/deployments/${f}`, here));
