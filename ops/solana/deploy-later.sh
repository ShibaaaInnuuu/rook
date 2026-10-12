#!/usr/bin/env bash
# Deploy Rook's programs at the ids reserved on day 1, paid from the community fund (decision D-g, docs/v9/solana-migration.md §2.9;
# runbook docs/v9/launch-day1-solana.md §4).
#
#   ops/solana/deploy-later.sh --keys <the day-1 keys dir> --launch <launch.json> --rpc <url> [--rpc2 <second url>]
#        [--ref <reviewed commit or tag>] [--fund-wallet <pubkey>] [--opt-level 3|s|z] [--priority-fee <micro-lamports per CU>]
#        [--send [--keep-authority] [--upgrade] [--yes]]
#
# 0. Preflight (audit A6-04; `rook-ceremony preflight`): the RPC's genesis is the launch file's cluster (mainnet-beta ⇒ Bitcoin
#    Mainnet, fixed for good at init_registry), the launch file is the pinned and reviewed one, the keys directory is day 1's
#    (<keys>/day1/day1-plan.json names these ids, this payer, this fund wallet), and the day-1 configs (one per listing, 17) ON CHAIN pay the claimer PDAs
#    recomputed from these keypairs (the ROOK config pays the fund wallet). The fund wallet must be the launch file's and day 1's (A6-08).
# 1. Builds rook_core and rook_backing (SBPF v3, `cargo build-sbf -- --locked`) from `git archive` of a NAMED commit (--ref; without
#    it HEAD, refused unless solana/ is clean: no uncommitted or untracked file reaches the build; A6-03) in <keys>/build/<commit>,
#    with the reserved ids swapped in (the five places that name them), checks that each binary carries its id, deletes the random
#    keypairs cargo-build-sbf writes next to the .so (A6-14), and records the commit and each .so's sha256 in
#    <keys>/build/<commit>/deploy-record.json. Prints the exact SOL (rent from the cluster): what stays locked and the peak while a
#    deploy buffer is open.
# 2. --send: the ceremony payer (<keys>/ceremony-payer.json, funded from the fund wallet by ONE plain transfer of the peak) deploys
#    rook_core then rook_backing as their upgrade authority through a buffer keypair kept in the job dir (a failed deploy resumes
#    with it; the CLI then prints no seed phrase, and any it prints is redacted; A6-10), then `solana program dump` of each must
#    equal the built .so byte for byte. A program already deployed with the same bytes is skipped; one deployed with OTHER bytes is
#    refused unless --upgrade (A6-03). Then `rook-ceremony init` runs init_registry (owner = the ceremony payer FOR THE SETUP ONLY,
#    the launch file's Bitcoin network, [rook_backing's converter PDA]) and init_backing, and reads both back. The fund wallet (a
#    Phantom / Ledger wallet) is never a key file: the ceremony payer signs the listing ceremony (`rook-ceremony list`), proposes the
#    fund wallet (`rook-ceremony propose-owner`), and the fund wallet accepts with ONE signature (`rook-ceremony accept-owner`);
#    `rook-ceremony verify-owner` must pass before anything is announced (runbook §4).
# 3. Handover (part of --send; --keep-authority to opt out, A6-08): the upgrade authority of both programs moves to the fund wallet
#    (a single key: decision D-g; both can move to a multisig later), and `solana program show` must then name it. The new authority
#    does not sign: Agave's `set-upgrade-authority` refuses a non-signing new authority unless --skip-new-upgrade-authority-signer-check
#    (checked on a local validator, Agave 4.1.2), so the flag stays.
#    The ceremony payer keeps its remainder for the listing ceremony and propose-owner; it goes back to the fund wallet after
#    verify-owner, and then the file is deleted.
#
# The fund wallet never signs here (its one signature is accept_owner, after the listing ceremony). Nothing is sent without --send and the typed confirmation. Secrets are never printed.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd -P)"
export PATH="$HOME/.local/share/solana/install/active_release/bin:$HOME/.cargo/bin:$PATH"
FUND_WALLET="GGgXnNgtY4ySfz8PLAAwkoh8Ztj7p6jrM9towSaa6Wuy"
KEYS="" LAUNCH="" RPC="" RPC2="" REF="" OPT="3" SEND="" KEEP="" UPGRADE="" YES="" PRIO=""
while [ $# -gt 0 ]; do
  case "$1" in
    --keys) KEYS="$2"; shift 2;;
    --launch) LAUNCH="$2"; shift 2;;
    --rpc) RPC="$2"; shift 2;;
    --rpc2) RPC2="$2"; shift 2;;
    --ref) REF="$2"; shift 2;;
    --fund-wallet) FUND_WALLET="$2"; shift 2;;
    --opt-level) OPT="$2"; shift 2;;
    --priority-fee) PRIO="$2"; shift 2;;
    --send) SEND=1; shift;;
    --handover) shift;;   # the handover is part of --send now (audit A6-08); kept so the old runbook line still parses
    --keep-authority) KEEP=1; shift;;
    --upgrade) UPGRADE=1; shift;;
    --yes) YES=1; shift;;
    *) echo "unknown argument $1 (see the header)"; exit 2;;
  esac
done
[ -n "$KEYS" ] && [ -n "$LAUNCH" ] && [ -n "$RPC" ] || { sed -n 2,33p "$0"; exit 2; }
[ -d "$KEYS" ] || { echo "refused: no keys directory $KEYS (ops/solana/ceremony-day1.sh makes it)"; exit 2; }
. "$ROOT/ops/solana/keys-guard.sh"
KEYS="$(keys_guard "$KEYS" "$ROOT")" || exit 2   # audit A6-01: resolved, by identity, any git work tree
case "$OPT" in 3|s|z) ;; *) echo "refused: --opt-level 3|s|z"; exit 2;; esac
case "$PRIO" in "") ;; *[!0-9]*) echo "refused: --priority-fee is an integer (micro-lamports per compute unit)"; exit 2;; *) [ "$PRIO" -le 1000000 ] || { echo "refused: --priority-fee above 1,000,000 micro-lamports per CU"; exit 2; };; esac
for f in rook_core-keypair.json rook_backing-keypair.json ceremony-payer.json day1/day1-plan.json; do [ -f "$KEYS/$f" ] || { echo "refused: no $KEYS/$f (ops/solana/ceremony-day1.sh makes it)"; exit 2; }; done
[ -f "$LAUNCH" ] || { echo "refused: no launch file $LAUNCH"; exit 2; }
PLAN="$KEYS/day1/day1-plan.json"
CORE="$(solana-keygen pubkey "$KEYS/rook_core-keypair.json")"
BACKING="$(solana-keygen pubkey "$KEYS/rook_backing-keypair.json")"
PAYER="$(solana-keygen pubkey "$KEYS/ceremony-payer.json")"
jget() { node -e 'const j=JSON.parse(require("fs").readFileSync(process.argv[1],"utf8"));let v=j;for(const k of process.argv[2].split("."))v=v==null?v:v[k];console.log(v==null?"":v)' "$1" "$2"; }
NET="$(jget "$LAUNCH" bitcoin_network)"
# A6-08: the handover target / registry owner is the launch file's fund wallet and day 1's, not merely "a funded account"
for w in "$(jget "$LAUNCH" meteora.genesis_rook.fund_wallet.value)" "$(jget "$PLAN" fund_wallet)"; do
  [ "$w" = "$FUND_WALLET" ] || { echo "refused: the fund wallet $FUND_WALLET is not the launch file's / day 1's ($w)"; exit 2; }
done
[ "$PAYER" != "$FUND_WALLET" ] || { echo "refused: the ceremony payer is the fund wallet"; exit 2; }
rpc() { curl -s -m 30 "$1" -H 'content-type: application/json' -d "$2"; }
GEN="$(rpc "$RPC" '{"jsonrpc":"2.0","id":1,"method":"getGenesisHash"}' | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>console.log(JSON.parse(s).result))')"
echo "cluster genesis $GEN; rook_core $CORE; rook_backing $BACKING; payer $PAYER; fund wallet $FUND_WALLET; bitcoin $NET; opt-level $OPT"

TOOL="$ROOT/ops/rehearsal/sol-devnet/tool"
(cd "$TOOL" && cargo build -q --release --bin rook-ceremony)
CER="$TOOL/target/release/rook-ceremony"
R2=(); [ -n "$RPC2" ] && R2=(--rpc2 "$RPC2")
# ---- 0. preflight (A6-04): cluster ⇔ launch file, keys = day 1's, the on-chain configs pay these keys' claimers ---------------------
"$CER" preflight --rook-core "$CORE" --rook-backing "$BACKING" --launch "$LAUNCH" --plan "$PLAN" --rpc "$RPC" \
  --fund-wallet "$FUND_WALLET" --payer "$KEYS/ceremony-payer.json" ${R2[@]+"${R2[@]}"}

# ---- 1. build from a pinned commit (A6-03) -------------------------------------------------------------------------------------------
if [ -z "$REF" ]; then
  REF=HEAD
  [ -z "$(git -C "$ROOT" status --porcelain --untracked-files=all -- solana)" ] \
    || { echo "refused: solana/ has uncommitted or untracked files and no --ref names the reviewed commit (git status -- solana)"; exit 2; }
fi
COMMIT="$(git -C "$ROOT" rev-parse --verify "$REF^{commit}")" || { echo "refused: --ref $REF is not a commit"; exit 2; }
B="$KEYS/build/$COMMIT"
mkdir -p "$B"
rm -rf "$B/solana" "$B/deploy"
git -C "$ROOT" archive --format=tar "$COMMIT" solana | tar -x -C "$B"
DEV_CORE="GepazuDZfS6zjwQ3U2zHCgk93xpRkp34AkmEFZKrAd7T" DEV_BACKING="8VqSJrKwQjFAXY5RXFxb69wYXmcCzTTuct4chu7FxbGf"
for f in programs/rook_core/src/lib.rs programs/rook_core/src/state/wake.rs programs/rook_backing/src/lib.rs programs/rook_backing/src/constants.rs Anchor.toml; do
  sed -i.bak -e "s/$DEV_CORE/$CORE/g" -e "s/$DEV_BACKING/$BACKING/g" "$B/solana/$f" && rm -f "$B/solana/$f.bak"
done
if grep -rq -e "$DEV_CORE" -e "$DEV_BACKING" "$B/solana/programs" --include='*.rs'; then echo "refused: a dev id is left in the build copy"; exit 1; fi
export CARGO_PROFILE_RELEASE_OPT_LEVEL="$OPT"
for p in rook_core rook_backing; do
  (cd "$B/solana" && cargo build-sbf --manifest-path "programs/$p/Cargo.toml" --arch v3 --sbf-out-dir "$B/deploy" -- --locked >"$B/$p.build.log" 2>&1) \
    || { echo "build of $p failed: $B/$p.build.log"; exit 1; }
done
rm -f "$B/deploy/"*-keypair.json   # A6-14: cargo-build-sbf writes RANDOM program keypairs here; only $KEYS/<p>-keypair.json is ours
python3 -I - "$B/deploy" "$CORE" "$BACKING" "$COMMIT" "$REF" "$OPT" <<'EOF'
import hashlib, json, sys, time
d, ids = sys.argv[1], {"rook_core": sys.argv[2], "rook_backing": sys.argv[3]}
A = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
def b58(s):
    n = 0
    for c in s: n = n * 58 + A.index(c)
    return n.to_bytes(32, "big")
rec = {"commit": sys.argv[4], "ref": sys.argv[5], "opt_level": sys.argv[6], "built_at": int(time.time()), "programs": {}}
for p, i in ids.items():
    so = open(f"{d}/{p}.so", "rb").read()
    assert so[48] == 3, f"{p}: not SBPF v3 (e_flags {so[48]})"
    assert b58(i) in so, f"{p}: the binary does not carry the id {i}"
    h = hashlib.sha256(so).hexdigest()
    rec["programs"][p] = {"id": i, "bytes": len(so), "sha256": h}
    print(f"{p}: {len(so)} bytes, SBPF v3, carries {i}, sha256 {h}")
json.dump(rec, open(f"{d}/../deploy-record.json", "w"), indent=2)
print(f"commit {rec['commit']} ({rec['ref']}): recorded in {d}/../deploy-record.json")
EOF

# ---- what is on chain already (a rerun): same bytes = skip; other bytes = refused unless --upgrade ----------------------------------
same_as_built() { # $1 program id, $2 .so → 0 identical, 1 different, 2 not deployed
  local out="$B/$(basename "$2" .so).dump"
  rm -f "$out"
  solana program dump "$1" "$out" -u "$RPC" -k "$KEYS/ceremony-payer.json" >/dev/null 2>&1 || return 2
  python3 -I -c 'import sys
a = open(sys.argv[1], "rb").read(); b = open(sys.argv[2], "rb").read()
sys.exit(0 if a[:len(b)] == b and not any(a[len(b):]) else 1)' "$out" "$2"
}
ST_rook_core="" ST_rook_backing=""   # bash 3.2 (macOS): no associative arrays
st() { eval "printf '%s' \"\$ST_$1\""; }
for p in rook_core rook_backing; do
  id="$CORE"; [ "$p" = rook_backing ] && id="$BACKING"
  set +e; same_as_built "$id" "$B/deploy/$p.so"; s=$?; set -e
  case $s in
    0) eval "ST_$p=same"; echo "$p: already deployed at $id with exactly these bytes: skipped";;
    1) eval "ST_$p=different"; echo "$p: DEPLOYED at $id with OTHER bytes than commit $COMMIT"
       [ -n "$UPGRADE" ] || { echo "refused: $p would be UPGRADED; pass --upgrade only for a reviewed upgrade"; exit 2; };;
    *) eval "ST_$p=absent";;
  esac
done

# ---- the exact SOL ----------------------------------------------------------------------------------------------------------------
rent() { rpc "$RPC" "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getMinimumBalanceForRentExemption\",\"params\":[$1]}" | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>console.log(JSON.parse(s).result))'; }
bal() { rpc "$RPC" "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getBalance\",\"params\":[\"$1\"]}" | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>console.log(JSON.parse(s).result.value))'; }
SC=$(stat -f %z "$B/deploy/rook_core.so" 2>/dev/null || stat -c %s "$B/deploy/rook_core.so")
SB=$(stat -f %z "$B/deploy/rook_backing.so" 2>/dev/null || stat -c %s "$B/deploy/rook_backing.so")
PA=$(rent 36); PDC=$(rent $((45 + SC))); PDB=$(rent $((45 + SB))); BUC=$(rent $((37 + SC))); BUB=$(rent $((37 + SB)))
# rook_core first: the peak is the larger of (core data + core buffer) and (core locked + backing data + backing buffer); the
# priority fee is budgeted at 200,000 CU per write transaction (an upper bound)
read -r LOCKED PEAK FEES <<<"$(python3 -c "
pa,pdc,pdb,buc,bub,sc,sb,prio=$PA,$PDC,$PDB,$BUC,$BUB,$SC,$SB,${PRIO:-0}
locked=2*pa+pdc+pdb
peak=max(pa+pdc+buc, pa+pdc+pa+pdb+bub)
ntx=-(-sc//1000)-(-sb//1000)+10
fees=ntx*5000+(ntx*200000*prio+999999)//1000000
print(locked,peak,fees)")"
sol() { python3 -c "print(f'{$1/1e9:.9f}')"; }
NEED=$((PEAK + FEES + 50000000))
echo "locked while deployed (a deposit: \`solana program close\` returns it to the upgrade authority, and closing ENDS every claimer's backing): $(sol $LOCKED) SOL  [rook_core $(sol $((PA + PDC))), rook_backing $(sol $((PA + PDB)))]"
echo "peak with a deploy buffer open (rook_core first): $(sol $PEAK) SOL; transaction fees ≤ $(sol $FEES) SOL${PRIO:+ (priority fee $PRIO µlamports/CU)}; init ≈ 0.01 SOL"
echo "the ceremony payer needs: $(sol $NEED) SOL (peak + fees + 0.05 margin) for the deploy, then the listing ceremony (it is the registry owner during the setup and pays it: \`node ops/solana/fund-status.mjs\`, ≈ 0.7 SOL); it holds $(sol "$(bal "$PAYER")"); the fund wallet holds $(sol "$(bal "$FUND_WALLET")")"
[ -n "$SEND" ] || { echo "(nothing sent: --send to deploy, init and hand over)"; exit 0; }

# ---- 2. deploy + verify + init ------------------------------------------------------------------------------------------------------
if [ "$(st rook_core)" = absent ] || [ "$(st rook_backing)" = absent ] || [ -n "$UPGRADE" ]; then
  [ "$(bal "$PAYER")" -ge "$NEED" ] || { echo "refused: fund the ceremony payer $PAYER with $(sol $NEED) SOL from the fund wallet first"; exit 1; }
fi
if [ -z "$YES" ]; then
  read -r -p "Deploy commit $COMMIT (rook_core $(st rook_core), rook_backing $(st rook_backing)) from $PAYER on genesis $GEN, then hand the upgrade authority to $FUND_WALLET$([ -n "$KEEP" ] && echo ' (NO: --keep-authority)')? Type 'deploy': " ok
  [ "$ok" = "deploy" ] || exit 1
fi
recovery() {
  echo
  echo "!! STOPPED after the deploy started. The upgrade authority may still be the HOT ceremony key $PAYER."
  echo "!! Rerun the same command (it skips what is deployed with these bytes and resumes from the buffer keypairs in $B), or hand over now:"
  echo "!!   solana program set-upgrade-authority <id> --upgrade-authority $KEYS/ceremony-payer.json --new-upgrade-authority $FUND_WALLET --skip-new-upgrade-authority-signer-check -k $KEYS/ceremony-payer.json -u <rpc>"
  echo "!! Open buffers are refunded with: solana program close --buffers -k $KEYS/ceremony-payer.json -u <rpc>"
}
trap recovery ERR
# A6-10: any 12-word seed phrase the CLI prints (an ephemeral buffer) is redacted; with --buffer <keypair file> it prints none
redact() { python3 -I -u -c 'import re, sys
for l in sys.stdin:
    sys.stdout.write("[a 12-word buffer seed phrase was printed here: redacted]\n" if re.fullmatch(r"\s*([a-z]+\s+){11}[a-z]+\s*", l) else l)'; }
PF=(); [ -n "$PRIO" ] && PF=(--with-compute-unit-price "$PRIO")
for p in rook_core rook_backing; do
  [ "$(st $p)" = same ] && continue
  id="$CORE"; [ "$p" = rook_backing ] && id="$BACKING"
  BUF="$B/$p-buffer-keypair.json"
  [ -f "$BUF" ] || (umask 077 && solana-keygen new --no-bip39-passphrase --silent --outfile "$BUF" >/dev/null)
  solana program deploy "$B/deploy/$p.so" --program-id "$KEYS/$p-keypair.json" --keypair "$KEYS/ceremony-payer.json" \
    --upgrade-authority "$KEYS/ceremony-payer.json" --buffer "$BUF" -u "$RPC" --max-sign-attempts 50 --use-rpc ${PF[@]+"${PF[@]}"} 2>&1 | redact
  same_as_built "$id" "$B/deploy/$p.so" || { echo "refused: the program now at $id is NOT the built $p.so of commit $COMMIT (solana program dump)"; false; }
  echo "$p: deployed at $id; on-chain bytes = the built .so (sha256 $(shasum -a 256 "$B/deploy/$p.so" | cut -d' ' -f1))"
done
"$CER" init --rook-core "$CORE" --rook-backing "$BACKING" --payer "$KEYS/ceremony-payer.json" --fund-wallet "$FUND_WALLET" \
  --launch "$LAUNCH" --plan "$PLAN" --rpc "$RPC" ${R2[@]+"${R2[@]}"} --send

# ---- 3. handover (A6-08) ------------------------------------------------------------------------------------------------------------
authority() { solana program show "$1" -u "$RPC" -k "$KEYS/ceremony-payer.json" --output json | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>console.log(JSON.parse(s).authority||"none"))'; }
if [ -n "$KEEP" ]; then
  trap - ERR
  echo "!! --keep-authority: the upgrade authority of both programs STAYS on the hot ceremony key $PAYER. Hand it over with the command below."
  recovery
  exit 0
fi
OWNER="$(rpc "$RPC" "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getAccountInfo\",\"params\":[\"$FUND_WALLET\"]}" | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{const v=JSON.parse(s).result.value;console.log(v?v.owner:"none")})')"
[ "$OWNER" = "11111111111111111111111111111111" ] || { echo "refused: the fund wallet $FUND_WALLET is not a funded system account ($OWNER)"; false; }
for id in "$CORE" "$BACKING"; do
  a="$(authority "$id")"
  if [ "$a" = "$FUND_WALLET" ]; then echo "$id: upgrade authority is already the fund wallet"; continue; fi
  [ "$a" = "$PAYER" ] || { echo "refused: the upgrade authority of $id is $a, neither the ceremony key nor the fund wallet"; false; }
  if [ -z "$YES" ]; then read -r -p "Move the upgrade authority of $id to $FUND_WALLET (a single key; check its last 6 characters: ${FUND_WALLET: -6})? Type 'handover': " ok; [ "$ok" = "handover" ] || false; fi
  solana program set-upgrade-authority "$id" --upgrade-authority "$KEYS/ceremony-payer.json" --new-upgrade-authority "$FUND_WALLET" \
    --skip-new-upgrade-authority-signer-check -k "$KEYS/ceremony-payer.json" -u "$RPC"
  a="$(authority "$id")"
  [ "$a" = "$FUND_WALLET" ] || { echo "refused: after the handover $id's upgrade authority reads $a"; false; }
  echo "$id: upgrade authority = $FUND_WALLET (read back)"
done
trap - ERR
echo "done. The ceremony payer $PAYER owns the registry FOR THE SETUP: keep it funded for the listing ceremony (fund-status.mjs, ≈ 0.7 SOL)."
echo "next, in one sitting (docs/v9/launch-day1-solana.md §4): rook-ceremony list --owner $KEYS/ceremony-payer.json --send,"
echo "  rook-ceremony propose-owner --send, the fund wallet's ONE signature (rook-ceremony accept-owner --print-tx → Phantom, or --ledger),"
echo "  rook-ceremony verify-owner; then: solana transfer --from $KEYS/ceremony-payer.json $FUND_WALLET ALL -u $RPC, and delete the file"
