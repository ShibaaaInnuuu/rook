/** Program-derived addresses, as the runtime derives them (ported from web-rook/src/rook/sol/pda.ts): the Metaplex metadata account of a mint. */
import { createHash } from "node:crypto";
import { ed25519 } from "@noble/curves/ed25519";
import { b58decode, b58encode } from "./bytes.js";
import { METADATA_PROGRAM } from "./dbc.js";

const isOnCurve = (b: Uint8Array) => { try { ed25519.ExtendedPoint.fromHex(b); return true; } catch { return false; } };
const MARKER = Buffer.from("ProgramDerivedAddress");

export function findProgramAddress(seeds: Uint8Array[], program: string): string {
  const prog = b58decode(program);
  for (let bump = 255; bump >= 0; bump--) {
    const h = new Uint8Array(createHash("sha256").update(Buffer.concat([...seeds.map((s) => Buffer.from(s)), Buffer.of(bump), Buffer.from(prog), MARKER])).digest());
    if (!isOnCurve(h)) return b58encode(h);
  }
  throw new Error("no program address");
}
export const metadataAddress = (mint: string): string => findProgramAddress([Buffer.from("metadata"), b58decode(METADATA_PROGRAM), b58decode(mint)], METADATA_PROGRAM);
