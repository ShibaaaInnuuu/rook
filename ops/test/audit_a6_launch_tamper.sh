#!/usr/bin/env bash
# Audit A6-05 (docs/v9/audit-2026-10/a6-launch-ops.md): rook-ceremony day1 builds PERMANENT DBC configs from fields of the launch
# file it never checks at run time (check_params / check_written cover a subset), and accepts a shortened or reordered
# listing_order, an unreviewed file (review.reviewed_by = null) and an empty ROOK uri.
#
#   bash ops/test/audit_a6_launch_tamper.sh     exit 0 = every tampered file refused (fixed); exit 1 = some were accepted
#
# Fixed 2026-10-10 (a6-launch-ops.md "Fixes"): the tampered files carry a filled-in review, so what refuses them is the pin
# (ops/solana/day1-pins.json, byte equality of the Borsh ConfigParameters; the listing order against nft-pilot.json). The review and
# ROOK-uri cases run with --send (no --rpc: nothing can be sent; the launch-file checks come first, and a refusal by the generic
# "--send needs --rpc" check does not count). A control (the untampered, reviewed file, dry run) must be ACCEPTED.
#
# Offline: no --rpc, so nothing is simulated or sent; throwaway keys in a temp directory.
set -uo pipefail
export PATH="$HOME/.local/share/solana/install/active_release/bin:$HOME/.cargo/bin:$PATH"
ROOT="$(cd "$(dirname "$0")/../.." && pwd -P)"
TOOL="$ROOT/ops/rehearsal/sol-devnet/tool"
(cd "$TOOL" && cargo build -q --release --bin rook-ceremony) || exit 2
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
(umask 077; solana-keygen new --no-bip39-passphrase --silent -o "$T/payer.json" >/dev/null)
CORE="$(solana-keygen pubkey "$(umask 077; solana-keygen new --no-bip39-passphrase --silent -o "$T/c.json" >/dev/null; echo "$T/c.json")")"
BACK="$(solana-keygen pubkey "$(umask 077; solana-keygen new --no-bip39-passphrase --silent -o "$T/b.json" >/dev/null; echo "$T/b.json")")"
node -e '
  const m = JSON.parse(require("child_process").execFileSync("node", [process.argv[1] + "/ops/solana/launch-params.mjs", "--meteora"]));
  const base = { cluster: "mainnet-beta", bitcoin_network: "Mainnet", listing_order: { listings: m.listing_order }, meteora: m.meteora,
                 review: { reviewed_by: "audit-a6-test" } };
  const fs = require("fs"), w = (n, f) => { const x = JSON.parse(JSON.stringify(base)); f(x); fs.writeFileSync(process.argv[2] + "/" + n + ".json", JSON.stringify(x)); };
  const cp = (x) => x.meteora.config_parameters;
  w("CONTROL-untampered-reviewed", () => {});
  w("SEND-untampered-unreviewed", (x) => { x.review.reviewed_by = null; });
  w("SEND-rook-uri-empty", (x) => { x.meteora.genesis_rook.metadata.uri.value = ""; });   // and --rook-uri "" (the default would apply otherwise)
  w("SEND-rook-uri-placeholder", (x) => { x.meteora.genesis_rook.metadata.uri.value = "https://…/rook.json"; });
  w("migrated_collect_fee_mode-1", (x) => { cp(x).migrated_pool_fee.collect_fee_mode.value = 1; });  // A3-02: coin-side LP fees stranded
  w("sqrt_start_price", (x) => { cp(x).sqrt_start_price.value = "134452599321738967"; });
  w("curve-liquidity", (x) => { cp(x).curve.value[0].liquidity = "215124158914782432327345953095693"; });
  w("partner_lp_unlocked", (x) => { cp(x).partner_liquidity_percentage.value = 50; cp(x).partner_permanent_locked_liquidity_percentage.value = 0; });
  w("locked_vesting", (x) => { cp(x).locked_vesting.amount_per_period.value = "1"; });
  w("rook-creator-share-50", (x) => { x.meteora.genesis_rook.overrides.creator_trading_fee_percentage.value = 50; });
  w("listing_order-symbol", (x) => { x.listing_order.listings[0].symbol = "wMIL"; });
  w("listing_order-collection", (x) => { x.listing_order.listings[1].collection = "0x0000000000000000000000000000000000000001"; });
  w("token_update_authority-0", (x) => { cp(x).token_update_authority.value = 0; });            // creators may rewrite metadata
  w("pool_creation_fee-100SOL", (x) => { cp(x).pool_creation_fee.value = "100000000000"; });   // every coin launch costs 100 SOL
  w("migrated_pool_fee-10pct", (x) => { cp(x).migrated_pool_fee.pool_fee_bps.value = 1000; });  // DAMM v2 pool fee 10 %
  w("migration_fee_option-0", (x) => { cp(x).migration_fee_option.value = 0; cp(x).migrated_pool_fee.pool_fee_bps.value = 0; });
  w("listing_order-3-entries", (x) => { x.listing_order.listings = x.listing_order.listings.slice(0, 3); });
  w("listing_order-frogs-first", (x) => { const l = x.listing_order.listings; const f = l.splice(11, 1)[0]; l.unshift(f); l.forEach((e, i) => e.listing_id = i); });
  w("listing_order-ordinals-swapped", (x) => { const l = x.listing_order.listings; [l[12], l[13]] = [l[13], l[12]]; l.forEach((e, i) => e.listing_id = i); });
  w("listing_order-ordinals-17-25-swapped", (x) => { const l = x.listing_order.listings; [l[17], l[25]] = [l[25], l[17]]; l.forEach((e, i) => e.listing_id = i); });
  // docs/v9/graduation-tiers.md: a listing moved to another tier, a tier curve swapped, a tier added, a tier threshold changed
  w("tier-bayc-to-85", (x) => { x.listing_order.listings.find((e) => e.slug === "bayc").graduation_sol = 85; });
  w("tier-milady-to-510", (x) => { x.listing_order.listings.find((e) => e.slug === "milady").graduation_sol = 510; });
  w("tier-170-curve-of-85", (x) => { x.meteora.tiers["170"].curve = x.meteora.tiers["85"].curve; x.meteora.tiers["170"].sqrt_start_price = x.meteora.tiers["85"].sqrt_start_price; });
  w("tier-340-added", (x) => { x.meteora.tiers["340"] = x.meteora.tiers["170"]; x.listing_order.listings.find((e) => e.slug === "bayc").graduation_sol = 340; });
  w("tier-510-threshold-500", (x) => { x.meteora.tiers["510"].migration_quote_threshold.value = "500000000000"; });
' "$ROOT" "$T"
accepted=0 control=0
for f in "$T"/*.json; do
  case "$f" in */payer.json|*/c.json|*/b.json) continue;; esac
  n="$(basename "$f" .json)"
  extra=()
  case "$n" in SEND-*) extra=(--send);; esac
  case "$n" in SEND-rook-uri-placeholder) ;; SEND-rook-uri-empty) extra+=(--rook-uri "");; SEND-*) extra+=(--rook-uri "ipfs://bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi");; esac
  if "$TOOL/target/release/rook-ceremony" day1 --rook-core "$CORE" --rook-backing "$BACK" --launch "$f" --out "$T/out-$n" --payer "$T/payer.json" ${extra[@]+"${extra[@]}"} >/dev/null 2>"$T/err"; then
    if [ "$n" = CONTROL-untampered-reviewed ]; then echo "accepted $n (control)"; control=1; else echo "ACCEPTED $n"; accepted=$((accepted + 1)); fi
  elif grep -q -e "--send needs --payer" -e "no answer" "$T/err"; then
    echo "NOT TESTED $n: refused only by: $(tail -1 "$T/err")"; accepted=$((accepted + 1))
  else
    echo "refused  $n: $(tail -1 "$T/err" | cut -c1-220)"
  fi
done
[ "$control" -eq 1 ] || { echo "the untampered, reviewed control file was refused: the test proves nothing"; accepted=$((accepted + 1)); }
echo "$accepted tampered/unreviewed launch file(s) accepted"
[ "$accepted" -eq 0 ]
