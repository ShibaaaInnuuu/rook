#!/usr/bin/env bash
# The Solana devnet rehearsal of Rook (docs/v9/rehearsal-sol-devnet.md). DEVNET, SIGNET/TESTNET4 and a THROWAWAY IC canister only:
# the tool refuses a mainnet-beta RPC, the canister signs with test_key_1, Ethereum is only read. No real funds move.
#
#   ops/rehearsal/sol-devnet/run.sh --local-validator [--dbc] [--keep-validator] [--launch <file>] [--no-build]
#       the dry run of the program half on a local solana-test-validator: the SBPF v3 binaries `anchor build` makes are deployed
#       (LiteSVM 0.10 cannot load them), the registry, three listings, the treasury and the desk initialized from a launch file,
#       the refusals simulated, the program drill run; --dbc clones Meteora DBC / DAMM v2 / Token Metadata from devnet into the
#       validator and runs a coin launch -> graduation -> `back` -> rook_core `convert` -> redeem in the same run (rook_backing's
#       ["converter"] PDA is allowed at init_registry: no 7-day wait). No IC, no devnet writes.
#
#   ops/rehearsal/sol-devnet/run.sh --local-replica [--no-build]     (a local replica must be running: cd ops/rehearsal/sol-devnet && dfx start --clean --background)
#       the canister half's dry run, free: the buyer built and installed on a LOCAL replica (PocketIC's key_1) with the rehearsal's
#       install argument (home = Solana(devnet)), its addresses, a tick against DEVNET (the genesis check through both providers, reads
#       at finalized), the frog sink and the 250 SOL ceiling (D-b), an upgrade carrying the state, the monitor's own sol_line read.
#       Nothing is written on devnet (the programs need not be deployed there).
#
#   ops/rehearsal/sol-devnet/run.sh --devnet --payer <keypair.json> (--subnet <id> | --next-to <canister id> | --subnet-type <type>)
#       [--cycles <n>] [--yes] [--job <dir>] [--launch <file>] [--frog-ids <file>] [--frog-offer <json>] [--rpc-urls <url>,<url>]
#       [--phases build,deploy,canister,programs,drill,drive,cleanup] [--keep-canister] [--dbc] [--skip-eth] [--fund-wait <s>]
#       [--eth-floor-sol 8] [--frog-floor-sol 1.5] [--no-build]
#       the whole rehearsal: the programs deployed to devnet with the OPERATOR's keypair (--payer: it becomes the programs' upgrade
#       authority and the registry's owner on devnet, for good: keep it for later runs), a throwaway buyer canister on IC mainnet
#       (test_key_1, home = Solana(devnet), two devnet providers), the listings naming it, the program drill, the canister checklist
#       (drive.py), and the canister deleted (ALWAYS, a trap, unless --keep-canister; no stop first: see the cleanup note).
#
#   ops/rehearsal/sol-devnet/run.sh --books --local-replica [--count 2] [--no-build] [--dry-run] [--payer <keypair.json>]
#   ops/rehearsal/sol-devnet/run.sh --books --devnet --payer <keypair.json> (--subnet <id> | --next-to <id> | --subnet-type <type>)
#       [--count 1-38] [--intent [--evidence <json>]] [--cycles <n>] [--yes] [--job <dir>] [--rpc-urls <url>,<url>] [--uri-base <url>]
#       [--phases build,canister,pins,fund,open,checklist,cleanup] [--keep-canister] [--fund-wait <s>] [--open-wait <s>] [--no-build]
#       W15, the canister-only rehearsal (docs/v9/rehearsal-sol-devnet.md §8; the W10 runbook docs/v9/ceremony-canister-only.md): NO
#       program. The buyer installed through the chunked upload with `home = Solana { cluster: "devnet", rook_core: "books" }` and the
#       launch set's Ethereum collections at `wrapper = module = "books"` (canister-init.mjs --books), its addresses read, the launch
#       pins made (cargo run --example launch_pins, the Solana ones with --sol-custodian = sol_address), `coins_pin`ned and each
#       requested (`coins_request_open`: a collection opens on its first request, operator decision 2026-10-12), K_main funded with the float (--payer: the solana CLI sends it,
#       else the operator does), the collections watched until each is Open, the checklist (books.py: genesis, the unfunded wait, the
#       openings landed and read back, books_view, one intent end to end with --intent, the monitor's books.mjs, the chunked upgrade,
#       cycles). --local-replica: free (PocketIC's key_1, devnet READS; with --payer the local canister opens on devnet too); --devnet:
#       a throwaway canister on IC mainnet with test_key_1, deleted by the trap unless --keep-canister. --dry-run prints every command
#       and runs none.
#
# Keys: the payer is the operator's (never generated here for devnet, never printed); the role keys (keeper, guardian, custodian,
# holder) are throwaway keypairs the tool creates 0600 in <job>/keys and never prints; the canister's keys are threshold keys. The job
# directory defaults to $TMPDIR, never the repository. Exit 0 only when every phase passed.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
export PATH="$HOME/.local/share/solana/install/active_release/bin:$HOME/.cargo/bin:$PATH"
DFX="${DFX:-$HOME/Library/Application Support/org.dfinity.dfx/bin/dfx}"; [ -x "$DFX" ] || DFX="$(command -v dfx || true)"
export DFX_WARNING=-mainnet_plaintext_identity
MODE=""; PAYER=""; JOB=""; LAUNCH=""; FROG_IDS=""; FROG_OFFER=""; WHERE=(); CYCLES="3000000000000"; YES=0; ANY_SUBNET=0; FUND_WAIT=600; NO_BUILD=0
KEEP_CAN=0; KEEP_VAL=0; DBC=0; SKIP_ETH=0; ETH_FLOOR=8; FROG_FLOOR=1.5
# W15 (--books): the canister-only rehearsal's own switches
BOOKS=0; COUNT=2; INTENT=0; EVIDENCE=""; DRY=0; OPEN_WAIT=1800; URI_BASE="https://userook.fun/wx/"; PHASES_SET=0
BOOKS_PHASES="build,canister,pins,fund,open,checklist,cleanup"
# The canister's two devnet providers. The IC's 13 replicas reach these IPv4-only hosts through shared egress proxies, so a keyless
# endpoint's per-IP rate limit decides whether its reads reach consensus: OnFinality's public endpoint (about one request a second
# per IP: HTTP 429 / -32029) split the replicas, "No consensus could be reached" on 82 reads in 40 ticks (run 3, 2026-10-09); ZAN's
# public endpoint answered every replica 429, so its reads agreed on {"http":429} and disagreed with the other provider (run 4).
# devnet.rpcpool.com is Triton's name for the same RPC as api.devnet.solana.com (one IP, one rate-limit bucket: 250 rps, 40 new
# connections / 10 s per IP): two names, ONE operator, so the agreement checks the canister's mechanics, not provider independence.
# A launch-grade pair needs keyed providers (Helius, Triton, ...): solana-migration.md §2.4.
SOL_URLS="https://api.devnet.solana.com,https://devnet.rpcpool.com"
PHASES="build,deploy,canister,programs,drill,drive,cleanup"
while [ $# -gt 0 ]; do
  case "$1" in
    --local-validator) MODE=local;;
    --local-replica) MODE=replica;;
    --devnet) MODE=devnet;;
    --payer) PAYER="${2:-}"; shift;;
    --job) JOB="${2:-}"; shift;;
    --launch) LAUNCH="${2:-}"; shift;;
    --frog-ids) FROG_IDS="${2:-}"; shift;;
    --frog-offer) FROG_OFFER="${2:-}"; shift;;
    --subnet) WHERE=(--subnet "${2:-}"); shift;;
    --next-to) WHERE=(--next-to "${2:-}"); shift;;
    --subnet-type) WHERE=(--subnet-type "${2:-}"); shift;;
    --any-subnet) ANY_SUBNET=1;;
    --cycles) CYCLES="${2:-}"; shift; echo "$CYCLES" | grep -Eq '^[0-9]+$' || { echo "refused: --cycles takes a whole number of cycles"; exit 2; };;
    --rpc-urls) SOL_URLS="${2:-}"; shift;;
    --phases) PHASES="${2:-}"; PHASES_SET=1; shift;;
    --books) BOOKS=1;;
    --count) COUNT="${2:-}"; shift; echo "$COUNT" | grep -Eq '^([1-9]|[12][0-9]|3[0-8])$' || { echo "refused: --count takes 1 to 38 (the collections the ceremony pins, in the canister's registry order)"; exit 2; };;
    --intent) INTENT=1;;
    --evidence) EVIDENCE="${2:-}"; shift;;
    --dry-run) DRY=1;;
    --open-wait) OPEN_WAIT="${2:-}"; shift;;
    --uri-base) URI_BASE="${2:-}"; shift;;
    --fund-wait) FUND_WAIT="${2:-}"; shift;;
    --eth-floor-sol) ETH_FLOOR="${2:-}"; shift;;
    --frog-floor-sol) FROG_FLOOR="${2:-}"; shift;;
    --no-build) NO_BUILD=1;;
    --keep-canister) KEEP_CAN=1;;
    --keep-validator) KEEP_VAL=1;;
    --dbc) DBC=1;;
    --skip-eth) SKIP_ETH=1;;
    --yes) YES=1;;
    *) echo "refused: unknown argument $1 (see the header of $0)"; exit 2;;
  esac; shift
done
has() { case ",$PHASES," in *",$1,"*) return 0;; *) return 1;; esac; }
if [ $BOOKS = 1 ]; then
  [ $PHASES_SET = 1 ] || PHASES="$BOOKS_PHASES"
  for p in $(echo "$PHASES" | tr ',' ' '); do case ",$BOOKS_PHASES," in *",$p,"*) ;; *) echo "refused: --books knows the phases $BOOKS_PHASES, not $p"; exit 2;; esac; done
  [ $INTENT = 1 ] && [ -z "$PAYER" ] && { echo "refused: --intent sends the intent's deposit from the payer: --payer <keypair.json>"; exit 2; }
  [ -n "$EVIDENCE" ] && [ ! -f "$EVIDENCE" ] && { echo "refused: --evidence $EVIDENCE is not a file"; exit 2; }
fi
case "$SOL_URLS" in *mainnet*) echo "refused: a Solana provider names mainnet ($SOL_URLS); this is the DEVNET rehearsal"; exit 2;; esac
STAMP="$(date -u +%Y%m%d-%H%M%S)"
fail=0
note() { echo "== $*"; }
bad() { echo "FAIL $*"; fail=1; }

case "$MODE" in
  local) JOB="${JOB:-${TMPDIR:-/tmp}/rook-sol-local-$STAMP}"; RPC="http://127.0.0.1:8899";;
  devnet)
    [ -n "$PAYER" ] && [ -f "$PAYER" ] || { echo "refused: --devnet needs --payer <keypair.json>: the operator's devnet key (the programs' upgrade authority and the registry's owner)"; exit 2; }
    case "$(cd "$(dirname "$PAYER")" && pwd)/" in "$ROOT"/*) echo "refused: the payer keypair is inside the repository ($PAYER); keep keys out of it"; exit 2;; esac
    JOB="${JOB:-${TMPDIR:-/tmp}/rook-sol-devnet-$STAMP}"; RPC="${SOL_URLS%%,*}"
    if has canister; then
      [ -x "$DFX" ] || [ $DRY = 1 ] || { echo "refused: dfx not found (set DFX)"; exit 2; }
      [ ${#WHERE[@]} -gt 0 ] || [ "$ANY_SUBNET" = 1 ] || { echo "refused: the canister phase needs a subnet: --subnet <id>, --next-to <canister id>, --subnet-type <type> or --any-subnet (the cycles ledger's default public subnets)"; exit 2; }
    fi;;
  replica)
    JOB="${JOB:-${TMPDIR:-/tmp}/rook-sol-replica-$STAMP}"; RPC="${SOL_URLS%%,*}"
    [ -x "$DFX" ] || [ $DRY = 1 ] || { echo "refused: dfx not found (set DFX)"; exit 2; }
    [ $DRY = 1 ] || ( cd "$HERE" && "$DFX" ping local >/dev/null 2>&1 ) || { echo "refused: no local replica; run: cd $HERE && $DFX start --clean --background"; exit 2; };;
  *) echo "refused: --local-validator, --local-replica or --devnet (see the header of $0)"; exit 2;;
esac
case "$JOB/" in "$ROOT"/*) echo "refused: the job directory is inside the repository ($JOB): it holds keypairs"; exit 2;; esac
mkdir -p "$JOB" && chmod 700 "$JOB"
echo "job: $JOB  rpc: $RPC  phases: $PHASES"
TOOL="$HERE/tool/target/debug/rook-devnet"

# ---- the throwaway canister's deletion (shared by both rehearsals) ------------------------------------------------------------------
VPID=""
stop_validator() { [ -n "$VPID" ] && [ $KEEP_VAL = 0 ] && { kill "$VPID" 2>/dev/null; wait "$VPID" 2>/dev/null; echo "validator stopped"; }; }
CREATED=0
cstate() { "$DFX" canister status rooksol --network ic 2>/dev/null | sed -n 's/^Status: //p'; }
delete_once() { ( cd "$HERE" && "$DFX" canister delete rooksol --network ic -y 2>&1 | grep -vE "Attempting to transfer|Not enough margin|dfx is deprecated" | sed 's/^/   /'; exit "${PIPESTATUS[0]}" ); }
cleanup() {
  stop_validator
  [ $CREATED = 1 ] || return 0
  if [ $KEEP_CAN = 1 ]; then echo "== the canister is KEPT (--keep-canister): $( cd "$HERE" && "$DFX" canister id rooksol --network ic). Delete it later: cd $HERE && $DFX canister delete rooksol --network ic -y"; return 0; fi
  echo "== clean up: withdraw what is left, delete"
  # sol-sign's fix (2026-10-09, measured on dfx 0.32): `dfx canister delete` on a RUNNING canister stops it, installs its temporary
  # wallet, withdraws and deletes; stopping it FIRST makes the withdrawal fail (IC0509/IC0508). So: no stop; on failure start it and retry.
  if delete_once; then echo "deleted: the canister is gone"
  else
    for _ in 1 2 3 4 5 6; do [ "$(cd "$HERE" && cstate)" = Running ] && break; ( cd "$HERE" && "$DFX" canister start rooksol --network ic >/dev/null 2>&1 ); sleep 5; done
    if delete_once; then echo "deleted on the second try (started, then deleted)"
    else echo "WARNING: the delete FAILED. By hand: cd $HERE && $DFX canister start rooksol --network ic; $DFX canister delete rooksol --network ic -y"; return 0; fi
  fi
  rm -f "$HERE/canister_ids.json"
  echo "cycles-ledger balance after: $("$DFX" cycles balance --network ic 2>&1)"
  CREATED=0
}

# ---- W15: the canister-only rehearsal (--books) ----------------------------------------------------------------------------------------
if [ $BOOKS = 1 ]; then
  [ "$MODE" = local ] && { echo "refused: --books is the canister-only rehearsal: --local-replica (free) or --devnet (a throwaway canister on the IC)"; exit 2; }
  run() { if [ $DRY = 1 ]; then echo "dry-run: $*"; else "$@"; fi; }              # a plain command
  runsh() { if [ $DRY = 1 ]; then echo "dry-run: $1"; else bash -c "$1"; fi; }     # a command line with cd / pipes
  WASM="$ROOT/autobuyer/canister-rs/target/wasm32-unknown-unknown/release/etch_autobuyer.wasm"; GZ="$JOB/rooksol.wasm.gz"
  if [ "$MODE" = replica ]; then NETW=local; KEY=key_1; export ROOK_LOCAL_REPLICA=1; else NETW=ic; KEY=test_key_1; export ROOK_LOCAL_REPLICA=0; fi
  SEED="rook-w15-$STAMP"
  MON=()
  if [ "$MODE" = replica ]; then PORT=$(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['networks']['local']['bind'].split(':')[1])" "$HERE/dfx.json"); MON=(--local --host "http://127.0.0.1:$PORT"); fi
  CID="<canister id>"
  cid() { if [ $DRY = 1 ]; then echo "$CID"; else ( cd "$HERE" && "$DFX" canister id rooksol --network "$NETW" ); fi; }
  canj() { if [ $DRY = 1 ] || [ ! -f "$JOB/canister.json" ]; then echo "<$1>"; else python3 -c "import json,sys; c=json.load(open(sys.argv[1])); print(c.get(sys.argv[2]) or '<' + sys.argv[2] + '>')" "$JOB/canister.json" "$1"; fi; }
  FLOAT=$(python3 -c "print((50_000_000 + $COUNT * 22_586_280 + 1_447_680 + 2_000_000 + 9_999_999) // 10_000_000 * 10_000_000 / 1e9)")
  echo "== W15, the canister-only rehearsal: $COUNT collection(s) of the launch set, phases $PHASES, $NETW ($KEY), providers $SOL_URLS$( [ $DRY = 1 ] && echo '; DRY RUN: every command printed, none run' )"
  echo "   the float K_main needs: $FLOAT SOL$( [ $INTENT = 1 ] && echo ' + the intent: 0.0515 SOL from the payer' ); cycles: the creation fee + ~0.3-1 T of ticks (devnet only)"
  books_cleanup() {
    [ $CREATED = 1 ] || return 0
    if [ $DRY = 1 ]; then echo "dry-run: $( [ $KEEP_CAN = 1 ] && echo 'the canister is KEPT (--keep-canister)' || echo "cd '$HERE' && '$DFX' canister delete rooksol --network $NETW -y (the trap: ALWAYS, also on failure or Ctrl-C)" )"; return 0; fi
    if [ "$MODE" = replica ]; then
      if [ $KEEP_CAN = 1 ]; then echo "== the local canister is KEPT (--keep-canister): $(cid)"; return 0; fi
      ( cd "$HERE" && "$DFX" canister delete rooksol --network local -y >/dev/null 2>&1; rm -f .dfx/local/canister_ids.json ); echo "local rooksol deleted"; CREATED=0
    else cleanup; fi
  }
  trap books_cleanup EXIT
  trap 'exit 130' INT TERM

  # build: the wasm (2.33 MB gzipped: over the 2 MiB single-message limit, hence the chunked install)
  if has build && [ $NO_BUILD = 0 ]; then
    note "build: the buyer canister (wasm32, release, --locked)"
    runsh "cd '$ROOT/autobuyer/canister-rs' && cargo build --locked --release --target wasm32-unknown-unknown -q" || { echo "FAIL the canister did not build"; exit 1; }
  fi

  # canister: create, the install argument, the chunked install, the addresses
  if has canister; then
    if [ -f "$WASM" ]; then
      gzip -n -9 -c "$WASM" > "$GZ"
      echo "buyer wasm: $(wc -c < "$WASM" | tr -d ' ') bytes, gzipped $(wc -c < "$GZ" | tr -d ' '), sha256 $(shasum -a 256 "$GZ" | cut -d' ' -f1)"
    else
      [ $DRY = 1 ] || { echo "FAIL $WASM is missing (the build phase)"; exit 1; }
      echo "dry-run: gzip -n -9 -c $WASM > $GZ"
    fi
    note "the install argument (canister-init.mjs --books: home = Solana { cluster devnet, rook_core \"books\" }, the Ethereum collections at wrapper = module = \"books\")"
    if [ $DRY = 1 ]; then echo "dry-run: ROOK_LOCAL_REPLICA=$ROOK_LOCAL_REPLICA node $HERE/canister-init.mjs --books $KEY $SOL_URLS signet $SEED --count $COUNT > $JOB/init.hex (then its raw bytes to $JOB/init.bin)"
    else
      node "$HERE/canister-init.mjs" --books "$KEY" "$SOL_URLS" signet "$SEED" --count "$COUNT" > "$JOB/init.hex" || { echo "FAIL the install argument"; exit 1; }
      python3 -c "import sys; open(sys.argv[2],'wb').write(bytes.fromhex(open(sys.argv[1]).read().strip()))" "$JOB/init.hex" "$JOB/init.bin"
    fi
    if [ "$MODE" = devnet ]; then
      if [ $DRY = 0 ]; then
        g=$(curl -s -m 20 "$RPC" -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"getGenesisHash"}' | python3 -c 'import sys,json;print(json.load(sys.stdin)["result"])')
        [ "$g" = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG" ] || { echo "refused: $RPC is not devnet (genesis $g)"; exit 2; }
        ME=$("$DFX" identity whoami); BAL=$("$DFX" cycles balance --network ic 2>&1) || { echo "refused: dfx cycles balance failed: $BAL"; exit 2; }
        cat <<EOF
This creates a canister on the IC (${WHERE[*]:-any default subnet}) with $CYCLES cycles from $ME's cycles-ledger balance ($BAL), installs the buyer
through the chunked upload (test_key_1, home = Solana(devnet) with rook_core = "books", no program), pins $COUNT collection(s), funds K_main
with $FLOAT devnet SOL from the payer, watches them open (the canister signs with the threshold Ed25519 test key: a wX mint + metadata
and a DBC config each, on devnet), reads the books, then deletes the canister and withdraws the rest. Ethereum MAINNET is READ only.
Expected spend: the creation fee (500 B on 13 nodes, ~1.3 T on 34) plus ~0.3-1 T of ticks and ~26 B a signature. No real money moves.
EOF
        if [ $YES != 1 ]; then read -r -p "Type 'rehearse' to go on: " a; [ "$a" = rehearse ] || { echo "stopped; nothing was created"; exit 2; }; fi
        if OLD=$(cd "$HERE" && "$DFX" canister id rooksol --network ic 2>/dev/null); then
          echo "refused: a rooksol canister from an earlier run still exists ($OLD). Delete it first: cd $HERE && $DFX canister delete rooksol --network ic -y"; exit 2
        fi
      fi
      note "create (${WHERE[*]:-any default subnet})"
      runsh "cd '$HERE' && '$DFX' canister create rooksol --network ic --with-cycles $CYCLES ${WHERE[*]:-} 2>&1 | sed 's/^/   /'"
      [ $DRY = 1 ] || ( cd "$HERE" && "$DFX" canister id rooksol --network ic >/dev/null 2>&1 ) || { echo "FAIL the canister was not created"; exit 1; }
    else
      [ $DRY = 1 ] || ! ( cd "$HERE" && "$DFX" canister id rooksol --network local >/dev/null 2>&1 ) || { echo "refused: a local rooksol exists; delete it: cd $HERE && $DFX canister delete rooksol --network local -y"; exit 2; }
      note "create on the local replica"
      runsh "cd '$HERE' && '$DFX' canister create rooksol --network local 2>&1 | grep -v deprecated | sed 's/^/   /'"
    fi
    CREATED=1
    echo "rooksol: $(cid)"
    note "the chunked install (autobuyer/scripts/chunked-install.sh: 1 MiB pieces, the module hash read back)"
    runsh "cd '$HERE' && bash '$ROOT/autobuyer/scripts/chunked-install.sh' --network $NETW --mode install $(cid) '$GZ' '$JOB/init.bin'" || { echo "FAIL the chunked install"; exit 1; }
    note "the addresses: K_main, K_fc (claimer), K_tre (treasury), M[i] (coll:i), the Ethereum and taproot custodies"
    run python3 -I "$HERE/books.py" addresses "$DFX" "$NETW" "$JOB" --count "$COUNT" || { echo "FAIL the canister's addresses"; exit 1; }
  fi

  # pins: the launch pins from the launch files, one coins_pin each
  if has pins; then
    note "the pins (cargo run --example launch_pins: the first $COUNT of the canister's registry, T_wX, Meteora's curve in wX, the value-based params)"
    runsh "cd '$ROOT/autobuyer/canister-rs' && cargo run -q --example launch_pins -- --evm-custody $(canj evm) --btc-custody $(canj taproot) --sol-custodian $(canj sol) --count $COUNT --uri-base '$URI_BASE' --out '$JOB/pins'" || { echo "FAIL the pins"; exit 1; }
    run python3 -I "$HERE/books.py" pins "$DFX" "$NETW" "$JOB" --count "$COUNT" || fail=1
  fi

  # fund: the float to K_main (the payer sends it, or the operator does)
  if has fund; then
    if [ "$MODE" = replica ] && [ -z "$PAYER" ]; then
      echo "== fund: skipped on the local replica without --payer (K_main $(canj sol) would need $FLOAT devnet SOL for the opening; pass --payer <keypair.json> to send it)"
    else
      note "fund: $FLOAT SOL to K_main $(canj sol)$( [ -n "$PAYER" ] && echo ' from the payer (solana transfer)' || echo ' by the operator (waiting)' )"
      run python3 -I "$HERE/books.py" fund "$DFX" "$NETW" "$JOB" --count "$COUNT" --rpc "$RPC" --fund-wait "$FUND_WAIT" ${PAYER:+--payer "$PAYER"} $( [ $INTENT = 1 ] && echo --intent ) || fail=1
    fi
  fi

  # open: watch the collections open (two signed transactions each, read back, listed in the books)
  if has open; then
    if [ "$MODE" = replica ] && [ -z "$PAYER" ]; then
      echo "== open: skipped on the local replica without --payer (nothing opens while K_main is under a collection's cost)"
    else
      note "open: ticks until every pinned collection is Open (≤ $OPEN_WAIT s; the reasons of every wait printed)"
      run python3 -I "$HERE/books.py" open "$DFX" "$NETW" "$JOB" --count "$COUNT" --rpc "$RPC" --open-wait "$OPEN_WAIT" || fail=1
    fi
  fi

  # checklist
  if has checklist; then
    note "the checklist (books.py checklist: 9 items, the report in $JOB/books-report.json)"
    run python3 -I "$HERE/books.py" checklist "$DFX" "$NETW" "$JOB" --count "$COUNT" --rpc "$RPC" --open-wait "$OPEN_WAIT" --wasm "$GZ" --arg "$JOB/init.hex" \
      --chunked "$ROOT/autobuyer/scripts/chunked-install.sh" ${PAYER:+--payer "$PAYER"} $( [ $INTENT = 1 ] && echo --intent ) ${EVIDENCE:+--evidence "$EVIDENCE"} ${MON[@]+"${MON[@]}"} || fail=1
  fi
  has cleanup || KEEP_CAN=1
  echo "(the job: $JOB — canister.json, init.hex, pins/, views.json, books-report.json)"
  [ $fail = 0 ] && echo "== PASS" || echo "== FAIL"
  exit $fail
fi

# ---- build ------------------------------------------------------------------------------------------------------------------------
if [ "$MODE" = replica ]; then
  if [ $NO_BUILD = 0 ]; then
    note "build: the buyer canister (wasm32, release, --locked) and the tool"
    ( cd "$ROOT/autobuyer/canister-rs" && cargo build --locked --release --target wasm32-unknown-unknown -q ) || { echo "FAIL the canister did not build"; exit 1; }
    ( cd "$HERE/tool" && cargo build -q 2>&1 | tail -5 ) || { echo "FAIL the tool did not build"; exit 1; }
  fi
  TOOL="$HERE/tool/target/debug/rook-devnet"
  [ -f "$JOB/plan-payer.json" ] || { solana-keygen new --no-bip39-passphrase --silent -o "$JOB/plan-payer.json" >/dev/null && chmod 600 "$JOB/plan-payer.json"; }
  "$TOOL" --rpc "$RPC" --job "$JOB" --payer "$JOB/plan-payer.json" plan > /dev/null || { echo "FAIL plan"; exit 1; }
  WASM="$ROOT/autobuyer/canister-rs/target/wasm32-unknown-unknown/release/etch_autobuyer.wasm"
  gzip -n -9 -c "$WASM" > "$JOB/rooksol.wasm.gz"
  GZ=$(wc -c < "$JOB/rooksol.wasm.gz" | tr -d ' ')
  echo "buyer wasm: $(wc -c < "$WASM" | tr -d ' ') bytes, gzipped $GZ ($(( GZ * 100 / 2097152 )) % of the 2 MiB install)"
  ROOK_LOCAL_REPLICA=1 node "$HERE/canister-init.mjs" "$JOB/plan.json" key_1 "$SOL_URLS" signet "rook-replica-$STAMP" > "$JOB/init.hex" || { echo "FAIL the install argument"; exit 1; }
  ( cd "$HERE" && "$DFX" canister id rooksol --network local >/dev/null 2>&1 ) && { echo "refused: a local rooksol exists; delete it: cd $HERE && $DFX canister delete rooksol --network local -y"; exit 2; }
  trap '( cd "$HERE" && "$DFX" canister delete rooksol --network local -y >/dev/null 2>&1; rm -f .dfx/local/canister_ids.json ); echo "local rooksol deleted"' EXIT
  ( cd "$HERE" && "$DFX" canister create rooksol --network local 2>&1 | grep -v deprecated | sed 's/^/   /' \
    && "$DFX" canister install rooksol --network local --mode install --wasm "$JOB/rooksol.wasm.gz" --argument "$(cat "$JOB/init.hex")" --argument-type raw -y 2>&1 \
    | grep -vE "deprecated|WARNING" | sed 's/^/   /' ) || { echo "FAIL the local install"; exit 1; }
  PORT=$(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['networks']['local']['bind'].split(':')[1])" "$HERE/dfx.json")
  python3 -I "$HERE/drive.py" smoke "$DFX" local "$JOB" --wasm "$JOB/rooksol.wasm.gz" --arg "$JOB/init.hex" --local --host "http://127.0.0.1:$PORT"
  exit $?
fi
if [ $NO_BUILD = 0 ] && { [ "$MODE" = local ] || has build; }; then
  note "build: the programs (anchor build: SBPF v3), the operator tool"
  ( cd "$ROOT/solana" && anchor build 2>&1 | tail -2 ) || { echo "FAIL anchor build"; exit 1; }
  ( cd "$HERE/tool" && cargo build -q 2>&1 | tail -5 ) || { echo "FAIL the tool did not build"; exit 1; }
  if [ "$MODE" = devnet ] && has canister; then
    note "build: the buyer canister (wasm32, release, --locked) and its gzip (the single-message install limit is 2 MiB)"
    ( cd "$ROOT/autobuyer/canister-rs" && cargo build --locked --release --target wasm32-unknown-unknown -q ) || { echo "FAIL the canister did not build"; exit 1; }
  fi
fi
for p in rook_core rook_backing; do
  so="$ROOT/solana/target/deploy/$p.so"
  [ -f "$so" ] || { echo "FAIL $so is missing (anchor build)"; exit 1; }
  arch=$(python3 -c "import sys; print(open(sys.argv[1],'rb').read()[48])" "$so")
  echo "$p.so: $(wc -c < "$so" | tr -d ' ') bytes, ELF e_flags $arch ($( [ "$arch" = 3 ] && echo 'SBPF v3' || echo "SBPF v$arch")), sha256 $(shasum -a 256 "$so" | cut -d' ' -f1)"
done
[ -x "$TOOL" ] || { echo "FAIL $TOOL is missing (build the tool: cd ops/rehearsal/sol-devnet/tool && cargo build)"; exit 1; }
T=("$TOOL" --rpc "$RPC" --job "$JOB")

# ---- the local validator -------------------------------------------------------------------------------------------------------------
VPID=""
stop_validator() { [ -n "$VPID" ] && [ $KEEP_VAL = 0 ] && { kill "$VPID" 2>/dev/null; wait "$VPID" 2>/dev/null; echo "validator stopped"; }; }
if [ "$MODE" = local ]; then
  curl -s -m 2 "$RPC" >/dev/null 2>&1 && { echo "refused: something already listens on $RPC (a validator left running?)"; exit 2; }
  CLONE=()
  if [ $DBC = 1 ]; then
    # Meteora's programs and the accounts DBC's migration needs, cloned from devnet (same ids as mainnet)
    CLONE=(--url https://api.devnet.solana.com --clone-upgradeable-program dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN
      --clone-upgradeable-program cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG --clone-upgradeable-program metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s
      --clone A8gMrEPJkacWkcb3DGwtJwTe16HktSEfvwtuDh2MCtck --maybe-clone FhVo3mqL8PW5pH5U2CN4XE33DokiyZnUwuGpH2hmHLuM
      --maybe-clone HLnpSz9h2S4hiLQ43rnSD9XkcUThA7B8hQMKmDaiTLcC)
  fi
  note "local validator ($(solana-test-validator --version | head -1))${CLONE:+, with Meteora DBC / DAMM v2 cloned from devnet}"
  solana-test-validator --reset --ledger "$JOB/ledger" --rpc-port 8899 --faucet-port 9900 --quiet ${CLONE[@]+"${CLONE[@]}"} > "$JOB/validator.log" 2>&1 &
  VPID=$!
  trap 'stop_validator' EXIT
  trap 'exit 130' INT TERM
  for _ in $(seq 1 60); do curl -s -m 2 "$RPC" -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' | grep -q '"ok"' && break; sleep 1; done
  curl -s -m 2 "$RPC" -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' | grep -q '"ok"' || { echo "FAIL the validator did not start (see $JOB/validator.log)"; exit 1; }
  PAYER="$JOB/payer.json"
  [ -f "$PAYER" ] || { solana-keygen new --no-bip39-passphrase --silent -o "$PAYER" >/dev/null && chmod 600 "$PAYER"; }
  solana -u "$RPC" airdrop 500 "$(solana-keygen pubkey "$PAYER")" >/dev/null || { echo "FAIL airdrop"; exit 1; }
  [ $DBC = 1 ] && solana -u "$RPC" -k "$PAYER" transfer --allow-unfunded-recipient FhVo3mqL8PW5pH5U2CN4XE33DokiyZnUwuGpH2hmHLuM 2 >/dev/null 2>&1
fi
T+=(--payer "$PAYER")
PAYER_PK="$(solana-keygen pubkey "$PAYER")"

# ---- deploy (both programs; the payer is the upgrade authority) --------------------------------------------------------------------
deploy_one() {  # <program> : deploy, or upgrade when the payer is its authority and the bytes differ
  local p="$1" so="$ROOT/solana/target/deploy/$1.so" kp="$ROOT/solana/target/deploy/$1-keypair.json" id auth
  id="$(solana-keygen pubkey "$kp")"
  auth="$(solana -u "$RPC" -k "$PAYER" program show "$id" 2>/dev/null | sed -n 's/^Authority: //p')"
  if [ -n "$auth" ]; then
    [ "$auth" = "$PAYER_PK" ] || { bad "$p ($id) is deployed on this cluster with authority $auth, not this payer ($PAYER_PK): use that payer"; return 1; }
    solana -u "$RPC" -k "$PAYER" program dump "$id" "$JOB/$p.onchain.so" >/dev/null 2>&1
    if cmp -s <(head -c "$(wc -c < "$so")" "$JOB/$p.onchain.so") "$so"; then echo "ok   $p ($id): the deployed bytes are this build's"; return 0; fi
    echo "     $p ($id): upgrading to this build"
  fi
  solana -u "$RPC" -k "$PAYER" program deploy --upgrade-authority "$PAYER" --program-id "$kp" "$so" --max-sign-attempts 30 \
    ${DEPLOY_FLAGS[@]+"${DEPLOY_FLAGS[@]}"} 2>&1 | tail -3 | sed 's/^/     /'
  [ "${PIPESTATUS[0]}" = 0 ] && echo "ok   $p deployed: $id" || { bad "$p deploy"; return 1; }
}
DEPLOY_FLAGS=()
[ "$MODE" = devnet ] && DEPLOY_FLAGS=(--with-compute-unit-price 50000)
if [ "$MODE" = local ] || has deploy; then
  note "deploy (payer $PAYER_PK: $(solana -u "$RPC" balance "$PAYER_PK" 2>/dev/null))"
  if [ "$MODE" = devnet ]; then
    g=$(curl -s -m 20 "$RPC" -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"getGenesisHash"}' | python3 -c 'import sys,json;print(json.load(sys.stdin)["result"])')
    [ "$g" = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG" ] || { echo "refused: $RPC is not devnet (genesis $g)"; exit 2; }
  fi
  deploy_one rook_core && deploy_one rook_backing || { echo "FAIL the deploy"; exit 1; }
fi

# ---- the launch file ------------------------------------------------------------------------------------------------------------------
if [ -z "$LAUNCH" ]; then
  LAUNCH="$JOB/launch.json"
  if [ ! -f "$LAUNCH" ]; then
    note "the launch file (ops/solana/launch-params.mjs: the live SOL/ETH rate, Bitcoin signet)"
    node "$ROOT/ops/solana/launch-params.mjs" --out "$LAUNCH" --cluster "$( [ "$MODE" = local ] && echo localnet || echo devnet )" --bitcoin-network signet \
      || { echo "FAIL the launch file (offline? pass --launch <file>)"; exit 1; }
  fi
fi
BTC_NET=$(python3 -c "import json,sys; print({'Signet':'signet','Testnet4':'testnet4','Mainnet':'mainnet'}[json.load(open(sys.argv[1]))['init']['init_registry']['bitcoin_network']])" "$LAUNCH")
[ "$MODE" = devnet ] && [ "$BTC_NET" = mainnet ] && { echo "refused: the devnet rehearsal's launch file names Bitcoin mainnet; generate one with --bitcoin-network signet"; exit 2; }
echo "launch file: $LAUNCH (Bitcoin $BTC_NET)"

# ---- keys, funding, the plan -----------------------------------------------------------------------------------------------------------
if [ "$MODE" = local ] || has canister || has programs || has drill || has drive; then
  note "role keys and the plan"
  "${T[@]}" keys | sed 's/^/     /'
  # the drill's 4 SOL and DBC's 3.5 only when this run has those phases (a rerun of the canister phases needs the floors' share only)
  DRILL_SOL=0; { [ "$MODE" = local ] || has drill; } && DRILL_SOL=1
  DBC_SOL=0; [ $DBC = 1 ] && { [ "$MODE" = local ] || has drill || has dbc; } && DBC_SOL=1
  HOLDER_SOL=$(python3 -c "print(round(${ETH_FLOOR}*1.9*$((1 - SKIP_ETH)) + ${FROG_FLOOR}*1.9 + 4*${DRILL_SOL} + 3.5*${DBC_SOL}, 2))")
  if [ "$MODE" = local ]; then "${T[@]}" fund --airdrop --sol-holder 40 || bad "fund"; else "${T[@]}" fund --sol-holder "$HOLDER_SOL" || bad "fund"; fi
  "${T[@]}" plan > "$JOB/plan.out" || { echo "FAIL plan"; exit 1; }
fi

# ---- the canister (IC mainnet, test_key_1; devnet only) ------------------------------------------------------------------------------
# (CREATED, cstate, delete_once and cleanup are defined above the books block: both rehearsals delete the canister the same way)
trap cleanup EXIT
trap 'exit 130' INT TERM
if [ "$MODE" = devnet ] && has canister; then
  WASM="$ROOT/autobuyer/canister-rs/target/wasm32-unknown-unknown/release/etch_autobuyer.wasm"
  [ -f "$WASM" ] || { echo "FAIL $WASM is missing (the build phase)"; exit 1; }
  gzip -n -9 -c "$WASM" > "$JOB/rooksol.wasm.gz"
  GZ=$(wc -c < "$JOB/rooksol.wasm.gz" | tr -d ' ')
  echo "buyer wasm: $(wc -c < "$WASM" | tr -d ' ') bytes, gzipped $GZ ($(( GZ * 100 / 2097152 )) % of the 2 MiB install), sha256 $(shasum -a 256 "$JOB/rooksol.wasm.gz" | cut -d' ' -f1)"
  [ "$GZ" -lt 2000000 ] || { echo "FAIL the gzipped module is at the 2 MiB single-message limit: the chunked install (decision D-d) is due"; exit 1; }
  ROOK_LOCAL_REPLICA=0 node "$HERE/canister-init.mjs" "$JOB/plan.json" test_key_1 "$SOL_URLS" "$BTC_NET" "rook-devnet-$STAMP" > "$JOB/init.hex" || { echo "FAIL the install argument"; exit 1; }
  ME=$("$DFX" identity whoami); BAL=$("$DFX" cycles balance --network ic 2>&1) || { echo "refused: dfx cycles balance failed: $BAL"; exit 2; }
  cat <<EOF
This creates a canister on the IC (${WHERE[*]:-any default subnet}) with $CYCLES cycles from $ME's cycles-ledger balance ($BAL), installs the buyer
(test_key_1, home = Solana(devnet), Bitcoin $BTC_NET), signs with the threshold Ed25519 test key and makes HTTPS outcalls to two Solana
DEVNET providers, two $BTC_NET explorers, public price tickers and two Ethereum MAINNET providers (READS only), then deletes it and
withdraws the rest. Expected spend: the creation fee (500 B on 13 nodes, ~1.3 T on 34) plus ~0.3-1 T of ticks. No real money moves.
EOF
  if [ $YES != 1 ]; then read -r -p "Type 'rehearse' to go on: " a; [ "$a" = rehearse ] || { echo "stopped; nothing was created"; exit 2; }; fi
  if OLD=$(cd "$HERE" && "$DFX" canister id rooksol --network ic 2>/dev/null); then
    echo "refused: a rooksol canister from an earlier run still exists ($OLD). Delete it first: cd $HERE && $DFX canister delete rooksol --network ic -y"; exit 2
  fi
  note "create (${WHERE[*]:-any default subnet}) and install"
  ( cd "$HERE" && "$DFX" canister create rooksol --network ic --with-cycles "$CYCLES" ${WHERE[@]+"${WHERE[@]}"} 2>&1 | sed 's/^/   /' )
  ( cd "$HERE" && "$DFX" canister id rooksol --network ic >/dev/null 2>&1 ) || { echo "FAIL the canister was not created"; exit 1; }
  CREATED=1
  echo "rooksol: $(cd "$HERE" && "$DFX" canister id rooksol --network ic)"
  ( cd "$HERE" && "$DFX" canister install rooksol --network ic --mode install --wasm "$JOB/rooksol.wasm.gz" --argument "$(cat "$JOB/init.hex")" --argument-type raw -y 2>&1 \
    | grep -v "dfx is deprecated" | sed 's/^/   /'; exit "${PIPESTATUS[0]}" ) || { echo "FAIL the install"; exit 1; }
  python3 -I "$HERE/drive.py" addresses "$DFX" ic "$JOB" || { echo "FAIL the canister's addresses"; exit 1; }
  echo "the canister's Solana address: $(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['sol'])" "$JOB/canister.json") (funded by the payer at checklist item 3, or at https://faucet.solana.com)"
fi

# ---- the programs: registry, listings, refusals ------------------------------------------------------------------------------------------
if [ "$MODE" = local ] || has programs; then
  note "init (launch file $LAUNCH)"
  CAN=()
  if [ -f "$JOB/canister.json" ]; then
    CAN=($(python3 -c "import json,sys; c=json.load(open(sys.argv[1])); print('--canister', c['sol'], *(['--canister-taproot', c['taproot']] if c.get('taproot') else []), *(['--canister-evm', c['evm']] if c.get('evm') else []))" "$JOB/canister.json"))
  elif [ "$MODE" = devnet ] && has drive; then
    echo "refused: the listings name the canister as custodian; run the canister phase first (or drop drive from --phases)"; exit 2
  fi
  "${T[@]}" init --launch "$LAUNCH" ${CAN[@]+"${CAN[@]}"} ${FROG_IDS:+--frog-ids "$FROG_IDS"} || bad "init"
  "${T[@]}" refusals || bad "refusals"
fi
if [ "$MODE" = local ] || has drill; then
  note "the program drill (listing drill: an operator key as custodian; every custodian and holder instruction on this cluster's binary)"
  "${T[@]}" drill || bad "drill"
fi
if [ $DBC = 1 ] && { [ "$MODE" = local ] || has drill || has dbc; }; then
  note "Meteora DBC on this cluster: config, launch, buy to the threshold, migration, back -> convert, redeem"
  if [ "$MODE" = local ]; then "${T[@]}" dbc-launch --no-wait || bad "dbc-launch"; else "${T[@]}" dbc-launch || bad "dbc-launch"; fi
fi

# ---- the canister's checklist --------------------------------------------------------------------------------------------------------------
if [ "$MODE" = devnet ] && has drive; then
  [ $CREATED = 1 ] || { echo "refused: the drive phase needs the canister this run created"; exit 2; }
  note "the canister's checklist (drive.py; ~20-60 min: finalization ~15-30 s a step, ticks every ~20 s)"
  python3 -I "$HERE/drive.py" checklist "$DFX" ic "$JOB" --tool "$(printf '%q ' "${T[@]}")" --wasm "$JOB/rooksol.wasm.gz" --arg "$JOB/init.hex" \
    --fund-wait "$FUND_WAIT" --eth-floor-sol "$ETH_FLOOR" --frog-floor-sol "$FROG_FLOOR" ${FROG_OFFER:+--frog-offer "$FROG_OFFER"} \
    $( [ $SKIP_ETH = 1 ] && echo --skip-eth ) || fail=1
fi

note "status and report"
"${T[@]}" status | sed 's/^/     /'
"${T[@]}" report > "$JOB/report.txt"; tail -1 "$JOB/report.txt"
grep -E "^FAIL" "$JOB/report.txt" && fail=1
echo "(the job: $JOB — state.json, report.txt$( [ -f "$JOB/drive-report.json" ] && echo ", drive-report.json"))"
[ $fail = 0 ] && echo "== PASS" || echo "== FAIL"
exit $fail
