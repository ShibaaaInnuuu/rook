import type { CapPolicyV, HeadroomV } from "./ic.js";

/**
 * docs/v5/launch-gates.md G7, the stage table, and the stage the canister's `custody_caps()` actually says. Pure.
 * A stage is EXACT when every collection cap and both global caps equal that stage's numbers; "at most n" when they fit under
 * stage n without equalling it; "above 3" otherwise. Per-item maxima are not compared: from stage 1 they are per collection
 * (≤ 1.2 × floor), not a table number.
 */
const E = 10n ** 18n;
export const STAGES = [
  { stage: 0, items: 3, value: 10n * E, globalItems: 6, globalValue: 10n * E },
  { stage: 1, items: 12, value: 40n * E, globalItems: 24, globalValue: 40n * E },
  { stage: 2, items: 48, value: 160n * E, globalItems: 96, globalValue: 160n * E },
  { stage: 3, items: 192, value: 640n * E, globalItems: 384, globalValue: 640n * E },
] as const;

/**
 * J7 (the operator, 2026-09-27; `custodycap.rs` FIRST_EVM_*): an Ethereum collection (`<chain>:0x…`) is at stage 0 with 15 ETH and at
 * most 24 items (its count is set per collection so 15 ETH fits at its floor), ×4 a stage after; and the global caps are the stage's
 * or the sum of the collections' caps when that is more, so the total never binds before the collections' own caps.
 */
export const EVM_STAGE0 = { items: 24, value: 15n * E } as const;
const isEvm = (key: string) => !key.startsWith("btc:");

export type Stage = { stage: number | null; exact: boolean; label: string };

export function stageOf(p: CapPolicyV): Stage {
  const sumItems = p.collections.reduce((a, c) => a + c.max_items, 0), sumValue = p.collections.reduce((a, c) => a + c.max_value_wei, 0n);
  const fits = (s: (typeof STAGES)[number], eq: boolean) => {
    const k = 4 ** s.stage;
    const gi = Math.max(s.globalItems, sumItems), gv = s.globalValue > sumValue ? s.globalValue : sumValue;
    return p.collections.every((c) => {
      if (isEvm(c.collection)) {
        const items = EVM_STAGE0.items * k, value = EVM_STAGE0.value * BigInt(k);
        return c.max_items <= items && (eq ? c.max_value_wei === value : c.max_value_wei <= value);
      }
      return eq ? c.max_items === s.items && c.max_value_wei === s.value : c.max_items <= s.items && c.max_value_wei <= s.value;
    }) && (eq ? (p.global_max_items === s.globalItems || p.global_max_items === gi) && (p.global_max_value_wei === s.globalValue || p.global_max_value_wei === gv)
      : p.global_max_items <= gi && p.global_max_value_wei <= gv);
  };
  for (const s of STAGES) if (fits(s, true)) return { stage: s.stage, exact: true, label: `stage ${s.stage}` };
  for (const s of STAGES) if (fits(s, false)) return { stage: s.stage, exact: false, label: `custom, at most stage ${s.stage}` };
  return { stage: null, exact: false, label: "above stage 3" };
}

export type CapsRead =
  | { kind: "read"; policy: CapPolicyV; headroom: HeadroomV[]; stage: Stage; stackStage: number; match: "equal" | "below" | "above"; at: number }
  | { kind: "error"; error: string; stackStage: number; at: number }
  | { kind: "absent"; note: string; stackStage: number; at: number };

/** The caps against the stack's stage: above it is red (G7: they must match), below is a note, unreadable is unverified. */
export function compareCaps(policy: CapPolicyV, headroom: HeadroomV[], stackStage: number, at: number): CapsRead {
  const stage = stageOf(policy);
  const match = stage.stage === null || stage.stage > stackStage ? "above" : stage.exact && stage.stage === stackStage ? "equal" : "below";
  return { kind: "read", policy, headroom, stage, stackStage, match, at };
}

export const capsStatus = (c: CapsRead): { status: "green" | "red" | "unverified"; reasons: string[] } =>
  c.kind === "read"
    ? c.match === "above" ? { status: "red", reasons: [`custody_caps() says ${c.stage.label}, above the stack's stage ${c.stackStage} (G7: they must match)`] }
      : { status: "green", reasons: c.match === "below" ? [`note: custody_caps() says ${c.stage.label}, under the stack's stage ${c.stackStage}`] : [] }
    : { status: "unverified", reasons: [c.kind === "error" ? `custody_caps() could not be read: ${c.error}` : c.note] };
