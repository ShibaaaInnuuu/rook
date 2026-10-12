#!/usr/bin/env bash
# Build the buyer and the timelock the way anyone can repeat, and print their SHA-256 — the value the IC reports as a canister's
# module hash when that exact file is installed. (docs/v2/timelock-controller.md, "Reproducible build".)
#
#   scripts/reproducible-build.sh            the canonical build, in Docker (linux/amd64, pinned image); wasm to autobuyer/build/
#   scripts/reproducible-build.sh native     the same commands on this machine (what the Dockerfile runs inside)
#   scripts/reproducible-build.sh verify <canister-id> <buyer|buyer.wasm.gz|timelock|timelock-rehearsal> [network]
#                                            compare a deployed canister's module hash with this source's build
#
# W14a: the buyer is also written gzipped (`gzip -9 -n`: no name, no time), the form it is installed in, through the IC's chunked code
# upload when it does not fit one message (scripts/chunked-install.sh, or the timelock's stage_chunk / propose_staged_upgrade). The module
# hash the IC reports is the SHA-256 of the file installed, so a gzipped install is verified against buyer.wasm.gz.
#
# The timelock is built twice: production (72 h floor) and with the `rehearsal` feature (10 s floor). They differ, on purpose.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
mode=${1:-docker}

build_native() {
  cd "$ROOT"
  : "${CARGO_HOME:=$HOME/.cargo}"
  # absolute paths otherwise end up inside the wasm (panic locations); remap both the source tree and cargo's registry
  export RUSTFLAGS="--remap-path-prefix=$ROOT=/build --remap-path-prefix=$CARGO_HOME=/cargo"
  mkdir -p build
  (cd canister-rs && cargo build --locked --release --target wasm32-unknown-unknown)
  cp canister-rs/target/wasm32-unknown-unknown/release/etch_autobuyer.wasm build/buyer.wasm
  (cd timelock-rs && cargo build --locked --release --target wasm32-unknown-unknown)
  cp timelock-rs/target/wasm32-unknown-unknown/release/etch_timelock.wasm build/timelock.wasm
  (cd timelock-rs && cargo build --locked --release --target wasm32-unknown-unknown --features rehearsal)
  cp timelock-rs/target/wasm32-unknown-unknown/release/etch_timelock.wasm build/timelock-rehearsal.wasm
  gzip -9 -n -c build/buyer.wasm > build/buyer.wasm.gz
  (cd build && sha256sum buyer.wasm buyer.wasm.gz timelock.wasm timelock-rehearsal.wasm 2>/dev/null || shasum -a 256 buyer.wasm buyer.wasm.gz timelock.wasm timelock-rehearsal.wasm)
  sizes
}

# W14a: one ingress message carries 2 MiB; the timelock's one-message proposal 2,000,000 bytes; above that the chunked path is the only one
sizes() {
  local gz; gz=$(wc -c < "$ROOT/build/buyer.wasm.gz" | tr -d ' ')
  echo "buyer.wasm.gz: $gz bytes ($(( gz * 100 / 2097152 )) % of one 2 MiB message)$( [ "$gz" -gt 1900000 ] && echo "; install it CHUNKED (scripts/chunked-install.sh, timelock stage_chunk)" )" >&2
}

case "$mode" in
  native) build_native ;;
  docker)
    cd "$ROOT"
    docker build --platform linux/amd64 -f Dockerfile.build -t etch-canisters-build . >&2
    rm -rf build && mkdir -p build
    id=$(docker create --platform linux/amd64 etch-canisters-build)
    docker cp "$id:/build/build/." build/ >/dev/null
    docker rm "$id" >/dev/null
    (cd build && shasum -a 256 buyer.wasm buyer.wasm.gz timelock.wasm timelock-rehearsal.wasm)
    sizes
    ;;
  verify)
    id=${2:?canister id}; which=${3:?buyer, buyer.wasm.gz, timelock or timelock-rehearsal}; net=${4:-ic}
    file="$ROOT/build/$which.wasm"; [ -f "$ROOT/build/$which" ] && file="$ROOT/build/$which"
    [ -f "$file" ] || { echo "build first: scripts/reproducible-build.sh" >&2; exit 2; }
    want=$(shasum -a 256 "$file" | cut -d' ' -f1)
    got=$(dfx canister --network "$net" info "$id" 2>/dev/null | awk '/Module hash/ {print $3}' | sed 's/^0x//')
    echo "built from source  $want"
    echo "deployed ($net)    ${got:-(none: empty or unreachable)}"
    [ "$want" = "$got" ] && echo "MATCH: $id runs this source's $which" || { echo "NO MATCH"; exit 1; }
    ;;
  *) echo "usage: $0 [docker|native|verify <id> <buyer|buyer.wasm.gz|timelock|timelock-rehearsal> [network]]" >&2; exit 2 ;;
esac
