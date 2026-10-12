// The independent runestone encoder, checked against constants worked out by hand from the protocol description.
//
// These tests are deliberately NOT a round trip. A round trip between an encoder and a decoder that share a convention
// passes whichever convention they share -- that is the shape of the `taproot_key()` bug, and `runestone.rs` already has
// round-trip tests of exactly that kind. What is checked here is the literal bytes, so that this module can be used to
// contradict the canister's Rust rather than agree with it by construction.
import test from "node:test";
import assert from "node:assert/strict";
import { bytesToHex } from "@noble/hashes/utils";
import { leb128, payload, script, parseRuneId, TAG_MINT, TAG_POINTER, TAG_BODY } from "../runestone.mjs";

test("LEB128 is little-endian base 128 with a continuation bit", () => {
  assert.equal(bytesToHex(leb128(0)), "00");
  assert.equal(bytesToHex(leb128(1)), "01");
  assert.equal(bytesToHex(leb128(127)), "7f", "the last value that fits in one byte");
  assert.equal(bytesToHex(leb128(128)), "8001", "and the first that does not");
  assert.equal(bytesToHex(leb128(255)), "ff01");
  assert.equal(bytesToHex(leb128(300)), "ac02");
  // 189444 = 4 + 72*128 + 11*128^2, which is the rune we are about to mint.
  assert.equal(bytesToHex(leb128(189_444)), "84c80b");
  assert.equal(bytesToHex(leb128(840_000)), "c0a233", "64 + 34*128 + 51*128^2 = 840000");
  // a u128 amount, because rune amounts are u128 and JS numbers are not
  assert.equal(bytesToHex(leb128(2n ** 64n)), "80808080808080808002", "nine empty groups of 7 bits, then 2 << 63");
  assert.throws(() => leb128(-1), /refused/);
});

test("a mint runestone is the Mint tag twice, then the pointer — hand-computed", () => {
  // Tag 20 (Mint) carries a rune id, which is two values, so the tag appears once per value.
  //   14        tag 20
  //   84 c8 0b  block 189444
  //   14        tag 20
  //   0f        tx 15
  //   16        tag 22 (Pointer)
  //   00        output 0
  assert.equal(bytesToHex(payload({ mint: "189444:15", pointer: 0 })), "1484c80b140f1600");
  //   6a        OP_RETURN
  //   5d        OP_13
  //   08        push 8 bytes
  assert.equal(bytesToHex(script({ mint: "189444:15", pointer: 0 })), "6a5d081484c80b140f1600");
});

test("an edict runestone is the Body tag then four varints per edict — hand-computed", () => {
  //   16 02        tag 22 (Pointer), output 2
  //   00           tag 0 (Body): everything after this is edicts
  //   84 c8 0b     block delta 189444
  //   0f           tx 15
  //   01           amount 1
  //   00           output 0
  assert.equal(
    bytesToHex(payload({ pointer: 2, edicts: [{ id: "189444:15", amount: 1, output: 0 }] })),
    "160200" + "84c80b" + "0f" + "01" + "00"
  );
  assert.equal(
    bytesToHex(script({ pointer: 2, edicts: [{ id: "189444:15", amount: 1, output: 0 }] })),
    "6a5d09" + "160200" + "84c80b0f0100"
  );
});

test("edict ids are deltas, so the encoder must sort and the second id is relative", () => {
  const p = bytesToHex(
    payload({
      edicts: [
        { id: "840000:5", amount: 20, output: 1 },
        { id: "840000:3", amount: 10, output: 1 },
      ],
    })
  );
  // body, then (840000, 3, 10, 1), then (delta 0, delta 2, 20, 1) — the second edict's tx is 5-3, not 5.
  assert.equal(p, "00" + "c0a233" + "03" + "0a" + "01" + "00" + "02" + "14" + "01");
  // across blocks the tx field is absolute, not a delta
  const q = bytesToHex(payload({ edicts: [{ id: "840000:3", amount: 1, output: 0 }, { id: "840001:7", amount: 1, output: 0 }] }));
  assert.ok(q.endsWith("01" + "07" + "01" + "00"), `block delta 1 then tx 7 absolute: ${q}`);
});

test("the body tag is last, because nothing can follow the edicts", () => {
  const p = payload({ mint: "1:1", pointer: 3, edicts: [{ id: "1:1", amount: 1, output: 0 }] });
  const hex = bytesToHex(p);
  const bodyAt = hex.indexOf("00", 0) / 2;
  const pointerAt = hex.indexOf("1603") / 2;
  assert.ok(pointerAt >= 0, "the pointer pair is present");
  assert.ok(pointerAt < hex.indexOf("00" + "01" + "01" + "01" + "00") / 2, "the pointer precedes the body");
  assert.equal(TAG_BODY, 0);
  assert.equal(TAG_MINT, 20);
  assert.equal(TAG_POINTER, 22);
  assert.ok(bodyAt >= 0);
});

test("the push form grows with the payload, and the script always starts OP_RETURN OP_13", () => {
  const make = (n) => Array.from({ length: n }, (_, i) => ({ id: `${800000 + i}:1`, amount: 1_000_000_000_000 + i, output: 1 }));
  const forms = [
    [1, 75, "a direct push"],
    [76, 255, "OP_PUSHDATA1"],
    [256, Infinity, "OP_PUSHDATA2"],
  ];
  for (const n of [2, 10, 40, 80]) {
    const edicts = make(n);
    const len = payload({ pointer: 2, edicts }).length;
    const s = script({ pointer: 2, edicts });
    assert.equal(s[0], 0x6a, "OP_RETURN");
    assert.equal(s[1], 0x5d, "OP_13");
    const [, , name] = forms.find(([lo, hi]) => len >= lo && len <= hi);
    const expected = { "a direct push": len, OP_PUSHDATA1: 76, OP_PUSHDATA2: 77 }[name];
    assert.equal(s[2], expected, `${len}-byte payload should use ${name}`);
  }
  assert.ok(payload({ pointer: 2, edicts: make(80) }).length > 255, "80 edicts really does cross into PUSHDATA2");
});

test("a rune id must be block:tx and nothing else", () => {
  assert.deepEqual(parseRuneId("189444:15"), { block: 189444n, tx: 15n });
  for (const bad of ["189444", "a:b", "", "189444:", ":15", "189444:15:2", " 189444:15"]) {
    assert.throws(() => parseRuneId(bad), /refused/, `${JSON.stringify(bad)} must refuse`);
  }
});
