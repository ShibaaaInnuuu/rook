import { checkTaproot } from "./bech32m";

/** Why the "post my address" button is not yet pressable, or null when it is. The same check the contract repeats on chain. */
export function destinationProblem(value: string): string | null {
  const v = value.trim();
  if (!v) return "Enter the address.";
  const c = checkTaproot(v);
  return c.ok ? null : `That address will not be accepted: ${c.why}`;
}
