// The state file: alert keys sent, txids judged, the 4663 cursor, the last certified values. Written atomically (a temporary
// file, then a rename), so a crash mid-write leaves the previous state, never half of one. It holds no secret.
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

export function loadState(path) {
  if (!existsSync(path)) return {};
  return JSON.parse(readFileSync(path, "utf8"));
}

export function saveState(path, state) {
  mkdirSync(dirname(path), { recursive: true });
  const tmp = `${path}.tmp-${process.pid}`;
  writeFileSync(tmp, JSON.stringify(state, (_, v) => (typeof v === "bigint" ? v.toString() : v), 1) + "\n");
  renameSync(tmp, path);
}
