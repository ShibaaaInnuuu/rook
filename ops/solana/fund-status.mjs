#!/usr/bin/env node
// The community deploy fund (decision D-g, docs/v9/solana-migration.md §2.9): how much of the deploy deposit is raised. READ-ONLY:
// it reads balances and one DBC account over JSON-RPC and never signs or sends anything.
//
//   node ops/solana/fund-status.mjs [--rpc https://api.mainnet-beta.solana.com] [--fund-wallet <pubkey>] [--plan <keys>/day1/day1-plan.json
//        | --rook-pool <pubkey>] [--core-bytes 1067856] [--backing-bytes 528184] [--json]
//
// Raised = the fund wallet's SOL + the ROOK creator (and partner) trading fees claimable in DBC (they belong to the fund wallet, which
// claims them with DBC claim_creator_trading_fee). Target = what ops/solana/deploy-later.sh needs at its peak: rent of both programs'
// ProgramData + program accounts + the larger open deploy buffer (rook_core first) + write-transaction fees + a 0.06 SOL margin
// (init and slack), from the cluster's own rent. Program sizes default to the opt-level-3 SBPF v3 builds of 2026-10-10 (programs commit 6588b81, reserved ids swapped in, `cargo build-sbf --locked` from git archive; table
// in docs/v9/launch-day1-solana.md); pass the sizes deploy-later.sh prints when they change. The `line` field is for the site.
import { readFileSync } from "node:fs";
import { listingOrder } from "./launch-params.mjs";

export const FUND_WALLET = "GGgXnNgtY4ySfz8PLAAwkoh8Ztj7p6jrM9towSaa6Wuy";
export const DBC = "dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN";
const LAMPORTS = 1_000_000_000n;
// DBC 0.2.1 VirtualPool (8-byte discriminator + PoolState, 424 bytes): partner_quote_fee @272, creator_quote_fee @360, is_migrated @305
const OFF = { partner_quote_fee: 272, creator_quote_fee: 360, is_migrated: 305, quote_reserve: 240 };
// The listing ceremony after the deploy (audit A6-11), paid by the registry owner during the setup = the ceremony payer (funded from the fund wallet; the fund wallet then accepts ownership with one signature): account data sizes (rent adds 128 bytes
// each). once: treasury, desk. Per listing: listing, wrapper, wX mint, item set, wX Metaplex metadata (max 679), module, module vault,
// module wX account, rook_backing partner. Pinned against the programs by the rook-ceremony unit test
// fund_status_listing_ceremony_sizes_are_the_programs.
export const LISTING_CEREMONY_SIZES = {"once":[2778,49],"per_listing":[285,662,82,4144,679,957,8,165,518]};
/** Metaplex's protocol fee per create_metadata (upper bound, lamports) and the listing ceremony's transactions per listing. */
export const METAPLEX_CREATE_FEE = 10_000_000n;
export const LISTING_TXS_PER_LISTING = 6n;
/** How many listings the ceremony makes: launch-params.mjs listingOrder() (nft-pilot.json: Ethereum, Bitcoin Frogs, `inscriptions[]`). */
export const LISTINGS = listingOrder().length;

function args(argv) {
  const o = {};
  for (let i = 0; i < argv.length; i++) {
    const k = argv[i];
    if (!k.startsWith("--")) throw new Error(`unknown argument ${k}`);
    o[k.slice(2)] = argv[i + 1] && !argv[i + 1].startsWith("--") ? argv[++i] : true;
  }
  return o;
}

async function rpc(url, method, params) {
  const r = await fetch(url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
    signal: AbortSignal.timeout(20_000) });
  const j = await r.json();
  if (j.error) throw new Error(`${method}: ${JSON.stringify(j.error)}`);
  return j.result;
}

/** Pure: the deploy target in lamports from the rent function (bytes → lamports) and the two program sizes. */
export function deployTarget(rent, coreBytes, backingBytes) {
  const pa = rent(36), pdc = rent(45 + coreBytes), pdb = rent(45 + backingBytes), buc = rent(37 + coreBytes), bub = rent(37 + backingBytes);
  const locked = 2n * pa + pdc + pdb;
  const peakA = pa + pdc + buc, peakB = pa + pdc + pa + pdb + bub;
  const peak = peakA > peakB ? peakA : peakB;
  const fees = BigInt(Math.ceil(coreBytes / 1000) + Math.ceil(backingBytes / 1000) + 10) * 5_000n;
  const margin = 60_000_000n;
  return { locked, peak, fees, margin, target: peak + fees + margin };
}

/** Pure: the listing ceremony's SOL (rent of every account it creates + Metaplex fees + one signature per transaction). */
export function listingTarget(rent, listings = LISTINGS) {
  const n = BigInt(listings);
  const once = LISTING_CEREMONY_SIZES.once.reduce((a, b) => a + rent(b), 0n);
  const per = LISTING_CEREMONY_SIZES.per_listing.reduce((a, b) => a + rent(b), 0n) + METAPLEX_CREATE_FEE;
  const fees = (2n + n * LISTING_TXS_PER_LISTING) * 5_000n;
  return { once, per_listing: per, fees, total: once + n * per + fees };
}

const sol = (l) => { const w = l / LAMPORTS, f = (l % LAMPORTS).toString().padStart(9, "0"); return `${w}.${f}`.replace(/\.?0+$/, "") || "0"; };
const u64 = (b, at) => b.readBigUInt64LE(at);

async function main() {
  const a = args(process.argv.slice(2));
  const url = a.rpc ?? "https://api.mainnet-beta.solana.com";
  const fund = a["fund-wallet"] ?? FUND_WALLET;
  const coreBytes = Number(a["core-bytes"] ?? 1_067_856), backingBytes = Number(a["backing-bytes"] ?? 528_184);
  const pool = a["rook-pool"] ?? (a.plan ? JSON.parse(readFileSync(a.plan, "utf8")).genesis_rook?.pool : null);

  // rent from the cluster: lamportsPerByte × (bytes + 128), read once from the rent of 0 bytes and of 1 byte
  const [r0, r1] = await Promise.all([rpc(url, "getMinimumBalanceForRentExemption", [0]), rpc(url, "getMinimumBalanceForRentExemption", [1])]);
  const perByte = BigInt(r1 - r0), base = BigInt(r0);
  const rent = (n) => base + perByte * BigInt(n);
  const t = deployTarget(rent, coreBytes, backingBytes);
  const lt = listingTarget(rent);
  const launchable = t.target + lt.total;

  const wallet = BigInt((await rpc(url, "getBalance", [fund, { commitment: "confirmed" }])).value);
  let fees = { creator: 0n, partner: 0n, migrated: null };
  if (pool) {
    const acc = (await rpc(url, "getAccountInfo", [pool, { encoding: "base64", commitment: "confirmed" }])).value;
    if (acc && acc.owner === DBC) {
      const b = Buffer.from(acc.data[0], "base64");
      fees = { creator: u64(b, OFF.creator_quote_fee), partner: u64(b, OFF.partner_quote_fee), migrated: b[OFF.is_migrated] === 1, quote_reserve: u64(b, OFF.quote_reserve) };
    }
  }
  const raised = wallet + fees.creator + fees.partner;
  const pct = Number((raised * 10_000n) / t.target) / 100;
  const pctLaunch = Number((raised * 10_000n) / launchable) / 100;
  const out = {
    fund_wallet: fund, rpc: url,
    raised_sol: sol(raised), target_sol: sol(t.target), percent: Math.min(pct, 100),
    line: `${sol(raised)} of ${sol(t.target)} SOL raised for Rook's deploy deposit`,
    launch_line: `${sol(raised)} of ${sol(launchable)} SOL raised for the deploy and the listing ceremony`,
    launchable_sol: sol(launchable), launchable_percent: Math.min(pctLaunch, 100),
    parts: { wallet_sol: sol(wallet), rook_creator_fees_claimable_sol: sol(fees.creator), rook_partner_fees_claimable_sol: sol(fees.partner),
      rook_pool: pool, rook_migrated: fees.migrated },
    target: { locked_deposit_sol: sol(t.locked), peak_with_buffer_sol: sol(t.peak), write_fees_sol: sol(t.fees), margin_sol: sol(t.margin),
      core_bytes: coreBytes, backing_bytes: backingBytes, lamports_per_byte: String(perByte),
      note: "The locked deposit comes back only by CLOSING the programs, which ends every claimer's backing for good, and it goes to the upgrade authority, not to contributors; the buffer part comes back at the end of each deploy." },
    listing_ceremony: { total_sol: sol(lt.total), once_sol: sol(lt.once), per_listing_sol: sol(lt.per_listing), fees_sol: sol(lt.fees), listings: LISTINGS,
      note: "After the deploy the ceremony payer (registry owner during the setup, funded from the fund wallet) pays treasury, desk and, per listing, listing + wrapper + wX mint + metadata + module + partner; the canister's own Solana costs are not included." },
    at: new Date().toISOString(),
  };
  if (a.json) console.log(JSON.stringify(out, null, 2));
  else {
    console.log(out.line + ` (${out.percent} %)`);
    console.log(`  wallet ${out.parts.wallet_sol} SOL + ROOK fees claimable ${out.parts.rook_creator_fees_claimable_sol} (creator) + ${out.parts.rook_partner_fees_claimable_sol} (partner)${pool ? "" : " (no --plan/--rook-pool: ROOK fees not read)"}`);
    console.log(`  target: peak ${out.target.peak_with_buffer_sol} + fees ${out.target.write_fees_sol} + margin ${out.target.margin_sol}; ${out.target.locked_deposit_sol} SOL stays locked after the deploy (returned only by closing the programs)`);
    console.log(`  + the listing ceremony ${out.listing_ceremony.total_sol} SOL (${LISTINGS} listings) → ${out.launch_line} (${out.launchable_percent} %)`);
  }
}

if (import.meta.url === `file://${process.argv[1]}`) main().catch((e) => { console.error(String(e.message ?? e)); process.exit(2); });
