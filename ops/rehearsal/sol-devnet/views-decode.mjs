// W15: a saved reply of the canister (hex: autobuyer/canister-rs/tests/fixtures/views/*.hex, written by its tests) decoded exactly as
// the monitor decodes it (monitor/src/candid.mjs `decodeReply`, plain JSON: nat64 as decimal strings, opt as [] or [x], blobs as hex),
// for books.py's parsers to read offline. Nothing is sent anywhere.
//   node ops/rehearsal/sol-devnet/views-decode.mjs <method> <file.hex>      → the plain JSON on stdout
const root = new URL("../../../", import.meta.url).pathname;
const { decodeReply, REPLY_TYPES } = await import(root + "monitor/src/candid.mjs");
const { readFileSync } = await import("node:fs");
const [method, file] = process.argv.slice(2);
if (!method || !file) throw new Error("usage: views-decode.mjs <method> <file.hex>");
if (!REPLY_TYPES[method]) throw new Error(`the monitor decodes no reply of ${method} (it knows ${Object.keys(REPLY_TYPES).join(", ")})`);
process.stdout.write(JSON.stringify(decodeReply(method, readFileSync(file, "utf8").trim())) + "\n");
