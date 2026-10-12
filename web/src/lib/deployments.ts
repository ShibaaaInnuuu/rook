/** docs/phase6.md. Which bundled deployment file is THE deployment, and which is a small real test. Pure, so it can be tested. */
export type Labelled = { chainId: number; label?: string };
export function pick<T extends Labelled>(files: readonly T[], chainId: number): { live: T | null; test: T | null } {
  // a file labelled "test" is NEVER the deployment, whatever chain id it carries; an unlabelled file is one written before labels existed
  const live = files.find((d) => d.chainId === chainId && (d.label === undefined || d.label === "mainnet")) ?? null;
  const test = files.find((d) => d.chainId === chainId && d.label === "test") ?? null;
  return { live, test };
}
export const netOf = (search: string): "staging" | "test" | null => { const n = new URLSearchParams(search).get("net"); return n === "staging" || n === "test" ? n : null; };
