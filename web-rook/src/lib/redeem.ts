import type { WrappedCollection } from "../api/types";
import { destinationProblem } from "./destination";

const ZERO = /^0x0{40}$/i, EVM = /^0x[0-9a-fA-F]{40}$/;
/** Why the destination will be refused, or null. The same checks the contracts make: taproot for inscriptions; not zero, not the custodian, for NFTs. */
export function redeemDestinationProblem(c: Pick<WrappedCollection, "kind" | "custody">, value: string): string | null {
  const v = value.trim();
  if (c.kind === "inscriptions") return destinationProblem(v);
  if (!v) return "Enter the address.";
  if (!EVM.test(v)) return "That is not an EVM address (0x followed by 40 hexadecimal characters).";
  if (ZERO.test(v)) return "The zero address would lose the item.";
  if (c.custody.kind === "evm" && v.toLowerCase() === c.custody.custodian.toLowerCase()) return "That is the custody address itself.";
  return null;
}

