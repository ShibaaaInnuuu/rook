import { stackFrom, type StackFile } from "./phase";

/** docs/v6/phase1-scope.md §8.3: the 4663 stack deployment file, bundled at build from src/deployments/stack-4663.json (copied by
 *  scripts/sync-content.mjs, by hand on purpose, as the v1 file is). lib/chain.ts reads the same folder and ignores it: its label is
 *  "stack", which `pick` never takes for the deployment. The site's live figures still come through the indexer, whose
 *  WRAPPED_CONFIG is this same file; the bundled copy is what a Phase-1 page is allowed to send to. */
const files = import.meta.glob("../deployments/stack-*.json", { eager: true, import: "default" }) as Record<string, unknown>;
export const bundledStack: { stack: StackFile | null; why: string | null } = stackFrom(Object.values(files));
