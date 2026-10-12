// A second, independent runestone encoder.
//
// It exists to disagree with the canister's. `canister-rs/src/runestone.rs` can encode and decode, and a round-trip test
// between those two halves passes whatever convention they share -- which is precisely the failure the signet rehearsal
// ran into with `taproot_key()`. So the bytes the canister produces are checked here against an implementation written
// from the protocol description rather than from the Rust, and the expected scripts below are hand-computed constants.
//
// This is the same move as recovering an EVM signature with `@noble/curves` when the canister recovered it with `k256`.
// Neither module is authority; their agreement is the evidence.

/// LEB128, the only integer encoding a runestone uses.
export function leb128(value) {
  let v = BigInt(value);
  if (v < 0n) throw new Error("refused: LEB128 here encodes no negative numbers");
  const out = [];
  for (;;) {
    const b = Number(v & 0x7fn);
    v >>= 7n;
    if (v === 0n) {
      out.push(b);
      return Uint8Array.from(out);
    }
    out.push(b | 0x80);
  }
}

export const TAG_BODY = 0;
export const TAG_MINT = 20;
export const TAG_POINTER = 22;

const OP_RETURN = 0x6a;
const OP_13 = 0x5d;

const cat = (...arrays) => {
  const out = new Uint8Array(arrays.reduce((n, a) => n + a.length, 0));
  let i = 0;
  for (const a of arrays) {
    out.set(a, i);
    i += a.length;
  }
  return out;
};

/// A minimal data push, then the bytes.
function push(data) {
  if (data.length === 0) return Uint8Array.from([0]);
  if (data.length <= 75) return cat(Uint8Array.from([data.length]), data);
  if (data.length <= 255) return cat(Uint8Array.from([76, data.length]), data);
  return cat(Uint8Array.from([77, data.length & 0xff, data.length >> 8]), data);
}

export const parseRuneId = (s) => {
  const m = /^(\d+):(\d+)$/.exec(s);
  if (!m) throw new Error(`refused: ${JSON.stringify(s)} is not a block:tx rune id`);
  return { block: BigInt(m[1]), tx: BigInt(m[2]) };
};

/// The payload for a plan. Every non-Body tag first, Body last -- there is no way back out of the body.
export function payload({ mint = null, pointer = null, edicts = [], unknownTag = null } = {}) {
  const parts = [];
  // An experiment affordance, not something the buyer ever emits. The Runes protocol's forward-compatibility rule is that
  // an UNRECOGNISED EVEN tag makes a runestone a cenotaph, while an odd one is ignored -- and `runestone.rs` ignores both,
  // though the doc comment on its `cenotaph` field says otherwise. Emitting one lets a real indexer settle which is right,
  // instead of this repository deciding from memory of a specification. See the probe in `runes-rehearsal.mjs`.
  if (unknownTag !== null) parts.push(leb128(unknownTag.tag), leb128(unknownTag.value ?? 0));
  if (mint !== null) {
    const id = parseRuneId(mint);
    parts.push(leb128(TAG_MINT), leb128(id.block), leb128(TAG_MINT), leb128(id.tx));
  }
  if (pointer !== null) parts.push(leb128(TAG_POINTER), leb128(pointer));
  if (edicts.length) {
    parts.push(leb128(TAG_BODY));
    const sorted = edicts
      .map((e) => ({ ...e, ...parseRuneId(e.id) }))
      .sort((a, b) => (a.block === b.block ? Number(a.tx - b.tx) : Number(a.block - b.block)));
    let lastBlock = 0n;
    let lastTx = 0n;
    for (const e of sorted) {
      const blockDelta = e.block - lastBlock;
      const txField = blockDelta === 0n ? e.tx - lastTx : e.tx;
      parts.push(leb128(blockDelta), leb128(txField), leb128(e.amount), leb128(e.output));
      lastBlock = e.block;
      lastTx = e.tx;
    }
  }
  return cat(...parts);
}

/// The full scriptPubKey.
export function script(plan) {
  const p = payload(plan);
  const chunks = [];
  for (let i = 0; i < p.length; i += 520) chunks.push(push(p.slice(i, i + 520)));
  return cat(Uint8Array.from([OP_RETURN, OP_13]), ...chunks);
}
