/** The other accounts the site reads: Metaplex token metadata (a coin's name, symbol, uri), an SPL token account / mint, Pyth's SOL/USD. */
import { b58encode } from "./base58";
import { i32At, i64At, u32At, u64At } from "./borsh";
import type { Rpc } from "./rpc";

export type TokenMeta = { updateAuthority: string; mint: string; name: string; symbol: string; uri: string };
/** Metaplex `Metadata` (key 4): key u8, update authority, mint, then three Borsh strings (padded with NULs by older versions). */
export function decodeMetadata(d: Uint8Array): TokenMeta {
  if (d[0] !== 4) throw new Error("not a Metaplex metadata account");
  let at = 65;
  const str = () => {
    const n = u32At(d, at); at += 4;
    if (n > 1024 || at + n > d.length) throw new Error("bad metadata string");
    const s = new TextDecoder().decode(d.subarray(at, at + n)); at += n;
    return s.replace(/\0+$/g, "").trim();
  };
  const name = str(), symbol = str(), uri = str();
  return { updateAuthority: b58encode(d.subarray(1, 33)), mint: b58encode(d.subarray(33, 65)), name, symbol, uri };
}

/** SPL token account (165 bytes): mint, owner, amount. */
export function decodeTokenAccount(d: Uint8Array): { mint: string; owner: string; amount: bigint } {
  if (d.length < 72) throw new Error("not a token account");
  return { mint: b58encode(d.subarray(0, 32)), owner: b58encode(d.subarray(32, 64)), amount: u64At(d, 64) };
}
/** SPL mint (82 bytes): supply @36, decimals @44. */
export function decodeMint(d: Uint8Array): { supply: bigint; decimals: number } {
  if (d.length < 82) throw new Error("not a mint");
  return { supply: u64At(d, 36), decimals: d[44] };
}

/** Pyth's SOL/USD price feed account on mainnet (the push oracle's sponsored feed, shard 0), and its feed id. */
export const PYTH_SOL_USD = "7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE";
export const PYTH_RECEIVER = "rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ";
export const SOL_USD_FEED_ID = "ef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d";
/** `PriceUpdateV2`: discriminator, write authority, verification level (Partial{u8} = 2 bytes | Full = 1 byte), then the message. */
export function decodePythPrice(d: Uint8Array): { feedId: string; price: number; conf: number; publishTime: number } {
  let at = 8 + 32;
  at += d[at] === 0 ? 2 : 1;
  const feedId = [...d.subarray(at, at + 32)].map((b) => b.toString(16).padStart(2, "0")).join(""); at += 32;
  const raw = i64At(d, at), conf = u64At(d, at + 8), expo = i32At(d, at + 16), publishTime = Number(i64At(d, at + 20));
  return { feedId, price: Number(raw) * 10 ** expo, conf: Number(conf) * 10 ** expo, publishTime };
}

export async function solUsd(rpc: Rpc): Promise<{ price: number; publishTime: number } | null> {
  try { const a = await rpc.account(PYTH_SOL_USD); if (!a) return null; const p = decodePythPrice(a.data); return p.price > 0 ? p : null; } catch { return null; }
}

