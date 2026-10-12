# docs/deploy/contracts-mainnet.md. Sourced by script/deploy.sh, script/deploy-stack.sh, script/deploy-wrapclaim.sh,
# script/deploy-nft.sh and script/list-auto.sh: the same refusals, in one place. It asks the RPC which chain it is BEFORE forge is
# started, never reads a private key or a password, and on Robinhood Chain (4663) requires a hardware wallet (--ledger / --trezor)
# or a Foundry keystore (--account <name>, docs/v6/phase1-scope.md §8.1) and the chain id typed by hand. With no RPC given it uses
# the local node on port 8604.
die() { echo "refused: $*" >&2; exit 1; }
LOCAL_RPC="http://127.0.0.1:8604"

# the key variables, and the three forge reads a signer or a password from by itself (a signer or a password nobody typed)
for v in PRIVATE_KEY ETH_PRIVATE_KEY DEPLOYER_PRIVATE_KEY MNEMONIC ETH_KEYSTORE ETH_KEYSTORE_ACCOUNT ETH_PASSWORD; do
  [ -z "${!v:-}" ] || die "$v is set in the environment. This script does not take a key; unset it."
done

# forge's --account reads $HOME/.foundry/keystores/<name> and nowhere else, whatever FOUNDRY_KEYSTORES_DIR says (measured
# 2026-09-25, docs/v6/phase1-scope.md §8.1). The file checked below is that file.
KEYSTORES="$HOME/.foundry/keystores"

# send_4663 <signer flag> <keystore name> <deployer>: the sender on Robinhood Chain, into SEND, or a refusal. A hardware wallet as
# before; or a Foundry keystore whose address is the deployer's. The password is typed into cast or forge, never into this script.
send_4663() {
  local signer="$1" account="$2" deployer="$3" ks addr lower here real
  lower=$(echo "$deployer" | tr 'A-F' 'a-f')
  case "$signer" in
    --ledger|--trezor) SEND=("$signer");;
    --account)
      echo "$account" | grep -Eq '^[A-Za-z0-9][A-Za-z0-9._-]*$' || die "the keystore name is letters, digits, dot, dash or underscore (and starts with a letter or digit)"
      if [ -n "${FOUNDRY_KEYSTORES_DIR:-}" ]; then
        here=$(cd "$FOUNDRY_KEYSTORES_DIR" 2>/dev/null && pwd -P || echo "$FOUNDRY_KEYSTORES_DIR"); real=$(cd "$KEYSTORES" 2>/dev/null && pwd -P || echo "$KEYSTORES")
        [ "$here" = "$real" ] || die "FOUNDRY_KEYSTORES_DIR is $FOUNDRY_KEYSTORES_DIR, but forge's --account reads $KEYSTORES whatever it says: unset it and keep the keystore in $KEYSTORES, so the file checked is the file that signs"
      fi
      ks="$KEYSTORES/$account"
      [ -f "$ks" ] || die "no keystore named '$account' in $KEYSTORES, the only place forge's --account reads. Make it there first: cast wallet import $account --interactive (or cast wallet new $KEYSTORES $account)"
      addr=$(python3 -c 'import json,sys; a=str(json.load(open(sys.argv[1])).get("address") or "").lower(); print(a if a.startswith("0x") or not a else "0x"+a)' "$ks" 2>/dev/null || true)
      if [ -z "$addr" ]; then
        echo "The keystore '$account' does not record its address, so cast opens it once to name it: type its password." >&2
        addr=$("$(command -v cast || echo "$HOME/.foundry/bin/cast")" wallet address --account "$account" 2>/dev/null | tr 'A-F' 'a-f' || true)
        echo "$addr" | grep -Eq '^0x[0-9a-f]{40}$' || die "could not read the address of the keystore '$account' (a wrong password, or not a keystore)"
      fi
      [ "$addr" = "$lower" ] || die "the keystore '$account' is for $addr, not the deployer $deployer"
      SEND=(--account "$account");;
    *) die "on chain 4663 the sender must be a hardware wallet (--ledger / --trezor) or a Foundry keystore (--account <name>)";;
  esac
}
# signer_words: how the confirmation prompt names the sender
signer_words() { case "$1" in --account) echo "the keystore '$2' (forge asks for its password)";; *) echo "the ${1#--}";; esac; }

# chain_of <rpc>: the chain id the RPC reports, in decimal
chain_of() {
  local hex
  hex=$(curl -s -m 15 -X POST -H 'content-type: application/json' --data '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}' "$1" \
    | sed -n 's/.*"result":"\(0x[0-9a-fA-F]*\)".*/\1/p' || true)
  [ -n "$hex" ] || die "the RPC at $1 did not report a chain id"
  echo $((hex))
}

# guard_chain <rpc> <chain>: the same allowlist as deploy.sh — 4663, 31337, or our staging fork from this machine
guard_chain() {
  local rpc="$1" chain="$2" staging host
  staging=$(sed -n 's/.*"chainId"[^0-9]*\([0-9][0-9]*\).*/\1/p' "$(dirname "${BASH_SOURCE[0]}")/../staging/chain-id.json" | head -1)
  [ -n "$staging" ] || die "staging/chain-id.json does not name a chain id"
  [ "$chain" != "46630" ] || die "chain 46630 is Robinhood's public testnet, not our staging fork ($staging): nothing here goes there"
  case "$chain" in 4663|31337|"$staging") ;; *) die "chain $chain is not one this script acts on (4663, 31337, or our staging fork $staging)";; esac
  if [ "$chain" = "$staging" ]; then
    host=$(echo "$rpc" | sed -E 's#^[a-z]+://([^/:]+).*#\1#')
    case "$host" in 127.0.0.1|localhost) ;; *) die "the staging fork is acted on from the machine that runs it, not through $host";; esac
  fi
  STAGING_ID="$staging"
}

is_ours() { [ "$1" = "31337" ] || [ "$1" = "$STAGING_ID" ]; }
