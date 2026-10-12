"""W14a: the plan of a chunked install, and the Candid argument files of its management-canister calls (chunked-install.sh).

    python3 -I chunked_plan.py <module> <arg.bin or ""> <canister> <install|reinstall|upgrade> <out dir>

Prints the plan (the full module's SHA-256: the module hash after the install; the 1 MiB pieces and their SHA-256) and writes
clear.did, chunk-<i>.did and install.did into <out dir>. The pieces are exactly the timelock's (`rules::install_plan`): 1,048,576 bytes
each, the last one shorter, concatenating to the module. No network.
"""
import hashlib
import os
import sys

IC_CHUNK_BYTES = 1_048_576          # the IC's chunk store: at most 1 MiB a chunk
SINGLE_INSTALL_BYTES = 1_900_000    # rules::SINGLE_INSTALL_BYTES: module + argument in one install_code
INGRESS_BYTES = 2 * 1024 * 1024     # one ingress message


def blob(b: bytes) -> str:
    return 'blob "' + "".join("\\%02x" % x for x in b) + '"'


def main() -> None:
    module, arg_path, canister, mode, out = sys.argv[1:6]
    wasm = open(module, "rb").read()
    if not (wasm.startswith(b"\0asm") or wasm.startswith(b"\x1f\x8b\x08")):
        sys.exit("refused: not a WebAssembly module and not gzip")
    arg = open(arg_path, "rb").read() if arg_path else b"DIDL\x00\x00"
    pieces = [(a, min(a + IC_CHUNK_BYTES, len(wasm))) for a in range(0, len(wasm), IC_CHUNK_BYTES)]
    hashes = [hashlib.sha256(wasm[a:b]).digest() for a, b in pieces]
    full = hashlib.sha256(wasm).hexdigest()
    print(f"module {module} {len(wasm)}")
    print(f"module_sha256 {full}")
    print(f"arg_bytes {len(arg)} arg_sha256 {hashlib.sha256(arg).hexdigest()}")
    print(f"one_message {'yes' if len(wasm) + len(arg) <= SINGLE_INSTALL_BYTES else 'no'} "
          f"(module + argument {len(wasm) + len(arg)} bytes; one install_code carries at most {SINGLE_INSTALL_BYTES}; ingress {INGRESS_BYTES})")
    print(f"pieces {len(pieces)}")
    for i, ((a, b), h) in enumerate(zip(pieces, hashes)):
        print(f"piece {i} {a} {b} {h.hex()}")
    assert b"".join(wasm[a:b] for a, b in pieces) == wasm
    target = f'principal "{canister}"'
    os.makedirs(out, exist_ok=True)
    with open(os.path.join(out, "clear.did"), "w") as f:
        f.write(f"(record {{ canister_id = {target} }})")
    for i, (a, b) in enumerate(pieces):
        with open(os.path.join(out, f"chunk-{i}.did"), "w") as f:
            f.write(f"(record {{ canister_id = {target}; chunk = {blob(wasm[a:b])} }})")
    m = {"install": "variant { install }", "reinstall": "variant { reinstall }",
         # upgrade, never skipping pre_upgrade: the buyer's state is saved and restored
         "upgrade": "variant { upgrade = opt record { skip_pre_upgrade = opt false } }"}[mode]
    hl = "; ".join(f"record {{ hash = {blob(h)} }}" for h in hashes)
    with open(os.path.join(out, "install.did"), "w") as f:
        f.write(f"(record {{ mode = {m}; target_canister = {target}; store_canister = null; chunk_hashes_list = vec {{ {hl} }}; "
                f"wasm_module_hash = {blob(bytes.fromhex(full))}; arg = {blob(arg)}; sender_canister_version = null }})")


if __name__ == "__main__":
    main()
