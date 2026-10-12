// The timelock's install argument (autobuyer/timelock-rs: `service : (Config)`, Config = record { target : principal; roles : Roles;
// delay_secs : nat64 }, Roles = record { proposers : vec principal; guardians : vec principal; veto_threshold : nat8 }) for the launch
// ceremony (docs/v9/ceremony-commands.md). The roles are the five keys of docs/v9/ceremony-canister-only.md §0 (O-7, made 2026-10-12):
// proposers p1, p2; guardians g1, g2, g3; veto 2 of 3; delay 72 h (the production build's floor). The target is the buyer canister
// (created first: the timelock is installed with its id, then the buyer's controllers become [timelock]).
//
//   node ops/ceremony/timelock-init.mjs <buyer canister id> [--bin <out.bin>]
//
// stdout: the Candid hex (dfx canister install … --argument-type raw --argument "$(cat timelock-init.hex)"); stderr: the argument decoded
// back, and the same in Candid text. Validated by `validateConfig`, a port of timelock-rs/src/rules.rs `Config::validate` and
// `Roles::validate` (production build: the `rehearsal` feature's relaxations are NOT applied), so a refusal here is the one the timelock
// would trap with at install.
import { writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

const ROOT = new URL("../../", import.meta.url).pathname;
const { IDL } = await import(ROOT + "monitor/node_modules/@icp-sdk/core/lib/esm/candid/index.js");
const { Principal } = await import(ROOT + "monitor/node_modules/@icp-sdk/core/lib/esm/principal/index.js");

/** docs/v9/ceremony-canister-only.md §0 (O-7): proposers p1, p2. */
export const PROPOSERS = ["jwtyr-bjz34-xpk3j-guwcn-igu4r-cdpw5-cybm6-xx5lt-nvrcm-qvlka-fqe", "jd5fp-gbdia-l5jyl-l64ph-nuwbd-sltlv-ruwnv-gxpyf-7yofw-ba5zw-sqe"];
/** Guardians g1 (the canister's pause guardian), g2, g3 (g3 the offline backup). */
export const GUARDIANS = ["4423w-dirhd-6mmzf-zrn3b-ukl6j-3p4mb-xhljt-gu67m-zojhm-yqdwk-fae", "ght7l-5bmui-5xas3-jjwzi-xm4ac-g57wd-hby3u-su46d-mefmb-ehiua-mae",
  "5re2m-u7frt-k642z-nzj7d-kskmn-67zi5-ewpsv-ghh7h-3bqkl-f5tz6-oqe"];
export const VETO_THRESHOLD = 2;
/** rules.rs MIN_DELAY_SECS (production): 72 h. The ceremony installs exactly the floor. */
export const MIN_DELAY_SECS = 72n * 3600n;
/** rules.rs MAX_ROLE. */
export const MAX_ROLE = 5;

const Roles = IDL.Record({ proposers: IDL.Vec(IDL.Principal), guardians: IDL.Vec(IDL.Principal), veto_threshold: IDL.Nat8 });
export const Config = IDL.Record({ target: IDL.Principal, roles: Roles, delay_secs: IDL.Nat64 });

const text = (p) => (typeof p === "string" ? p : p.toText());

/** rules.rs `Roles::validate`, in its order and with its messages. `rehearsal`: the feature build's relaxed rules (only for the ported
 *  tests; the ceremony never passes it). */
export function validateRoles(r, { rehearsal = false } = {}) {
  const anon = Principal.anonymous().toText();
  const proposers = r.proposers.map(text), guardians = r.guardians.map(text);
  const [p, g, k] = [proposers.length, guardians.length, r.veto_threshold];
  if (p === 0 || g === 0) throw new Error("refused: at least one proposer and one guardian");
  if (p > MAX_ROLE || g > MAX_ROLE) throw new Error(`refused: at most ${MAX_ROLE} proposers and ${MAX_ROLE} guardians`);
  if (proposers.includes(anon) || guardians.includes(anon)) throw new Error("refused: the anonymous principal cannot be a proposer or a guardian");
  const twice = (v) => v.some((x, i) => v.slice(0, i).includes(x));
  if (twice(proposers) || twice(guardians)) throw new Error("refused: a principal listed twice in one role");
  if (!Number.isInteger(k) || k < 0 || k > 255) throw new Error(`refused: veto_threshold ${k} is not a nat8`);
  if (k === 0 || k > g) throw new Error(`refused: a veto threshold of ${k} with ${g} guardians`);
  if (!rehearsal) {
    if (p < 2) throw new Error("refused: a single proposer, lost, would mean no upgrade ever again; name two at least");
    if (g < 3) throw new Error("refused: fewer than three guardians; a veto that needs two must survive losing one");
    if (k < 2 || k > g - 1) throw new Error(`refused: a veto threshold of ${k} with ${g} guardians; it must be at least 2 (no single guardian can freeze the buyer) and at most ${g - 1} (losing one guardian must not lose the veto)`);
    if (proposers.some((x) => guardians.includes(x))) throw new Error("refused: a principal is both a proposer and a guardian, so part of the veto would be the proposer's own");
  }
  return true;
}

/** rules.rs `Config::validate` (production floor). */
export function validateConfig(c, { rehearsal = false, minDelay = rehearsal ? 10n : MIN_DELAY_SECS } = {}) {
  if (text(c.target) === Principal.anonymous().toText()) throw new Error("refused: the anonymous principal cannot be the target");
  if (BigInt(c.delay_secs) < minDelay) throw new Error(`refused: a delay of ${c.delay_secs} s is below this build's floor of ${minDelay} s`);
  return validateRoles(c.roles, { rehearsal });
}

/** The ceremony's Config for `target` (the buyer's canister id), validated. */
export function timelockConfig(target) {
  if (typeof target !== "string" || /[<>]/.test(target)) throw new Error("refused: the target is a placeholder; give the buyer's canister id");
  let t;
  try { t = Principal.fromText(target.trim()); } catch (e) { throw new Error(`refused: target ${JSON.stringify(target)} is not a principal: ${e.message}`); }
  const raw = t.toUint8Array();
  // a canister id is an opaque id: 10 bytes, its last byte 0x01 (a user's self-authenticating principal ends 0x02)
  if (raw.length !== 10 || raw[9] !== 0x01) throw new Error(`refused: target ${t.toText()} is not a canister id (the buyer's)`);
  if ([...PROPOSERS, ...GUARDIANS].includes(t.toText())) throw new Error("refused: the target is a role holder");
  const c = { target: t, roles: { proposers: PROPOSERS.map((x) => Principal.fromText(x)), guardians: GUARDIANS.map((x) => Principal.fromText(x)),
    veto_threshold: VETO_THRESHOLD }, delay_secs: MIN_DELAY_SECS };
  // every role principal reads back as itself (a typo in the constants would change its checksum and fail fromText above)
  for (const x of [...PROPOSERS, ...GUARDIANS]) if (Principal.fromText(x).toText() !== x) throw new Error(`a role principal does not round-trip: ${x}`);
  validateConfig(c);
  return c;
}

/** Hex of the Config, decoded back and compared. */
export function encodeConfig(c) {
  const hex = Buffer.from(IDL.encode([Config], [c])).toString("hex");
  const [d] = IDL.decode([Config], Uint8Array.from(Buffer.from(hex, "hex"))); // a copy: a pooled Buffer's ArrayBuffer is not its bytes
  const same = d.target.toText() === text(c.target) && d.delay_secs === BigInt(c.delay_secs) && d.roles.veto_threshold === c.roles.veto_threshold
    && d.roles.proposers.map(text).join() === c.roles.proposers.map(text).join() && d.roles.guardians.map(text).join() === c.roles.guardians.map(text).join();
  if (!same) throw new Error("the timelock Config does not decode back to itself");
  validateConfig(d);
  return { hex, decoded: d };
}

/** The Candid text form (for reading, and for `dfx canister install --argument`). */
export function candidText(c) {
  const ps = (v) => v.map((x) => `principal "${text(x)}"`).join("; ");
  return `(record { target = principal "${text(c.target)}"; roles = record { proposers = vec { ${ps(c.roles.proposers)} }; `
    + `guardians = vec { ${ps(c.roles.guardians)} }; veto_threshold = ${c.roles.veto_threshold} : nat8 }; delay_secs = ${c.delay_secs} : nat64 })`;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const argv = process.argv.slice(2);
    const i = argv.indexOf("--bin");
    const bin = i >= 0 ? argv.splice(i, 2)[1] : null;
    if (argv.length !== 1) throw new Error("usage: node ops/ceremony/timelock-init.mjs <buyer canister id> [--bin <out.bin>]");
    const { hex, decoded } = encodeConfig(timelockConfig(argv[0]));
    if (bin) writeFileSync(bin, Buffer.from(hex, "hex"));
    process.stdout.write(hex + "\n");
    process.stderr.write(`The timelock's Config, decoded back (rules.rs Config::validate passes, production floor):\n  ${candidText(decoded)}\n`
      + `  proposers p1, p2; guardians g1 (the canister's pause guardian), g2, g3; veto ${decoded.roles.veto_threshold} of ${decoded.roles.guardians.length}; delay ${decoded.delay_secs} s (72 h)\n`);
  } catch (e) {
    process.stderr.write(String(e.message ?? e) + "\n");
    process.exit(1);
  }
}
