# Sourced by ceremony-day1.sh and deploy-later.sh (audit A6-01, docs/v9/audit-2026-10/a6-launch-ops.md).
#
#   keys_guard <keys path> <repository root>     prints the fully resolved keys path, or exits 2 with "refused: … inside the repository …"
#
# The keys directory holds the reserved program keypairs (control of every claimer PDA) and must never be inside a git work tree,
# where a routine `git add -A` stages it. "Inside" is decided by identity, not by spelling:
#   1. the whole path is resolved (python os.path.realpath: every symlink, the leaf included; a not-yet-created tail is kept as given);
#   2. refused if any existing ancestor of the resolved path is the repository root by inode (st_dev, st_ino) — macOS volumes are
#      case-insensitive, so /Users/x/Documents/rook and …/Rook are the same directory and a string compare misses it;
#   3. refused if the path compares inside the repository case-insensitively (the same check without the filesystem);
#   4. refused if `git rev-parse` succeeds from the nearest existing ancestor: inside ANY git work tree, this one or another
#      (GIT_DIR / GIT_WORK_TREE / GIT_CEILING_DIRECTORIES are cleared so the environment cannot hide or fake one).
keys_guard() {
  local keys="$1" root="$2" out
  out="$(python3 -I - "$keys" "$root" <<'PY'
import os, subprocess, sys
keys, root = sys.argv[1], sys.argv[2]
p = os.path.realpath(os.path.abspath(os.path.expanduser(keys)))
r = os.path.realpath(root)
rs = os.stat(r)
anc = p
while not os.path.exists(anc):
    nxt = os.path.dirname(anc)
    if nxt == anc:
        break
    anc = nxt
def refuse(why):
    print(f"refused: {keys} (resolved {p}) is inside the repository or a git work tree: {why}. It will hold the program keypairs; use a directory outside every repository, e.g. ~/rook-keys")
    sys.exit(2)
a = anc
while True:
    try:
        s = os.stat(a)
        if (s.st_dev, s.st_ino) == (rs.st_dev, rs.st_ino):
            refuse(f"{a} is the repository root {r} (same inode)")
    except OSError:
        pass
    nxt = os.path.dirname(a)
    if nxt == a:
        break
    a = nxt
if (p.casefold() + "/").startswith(r.casefold().rstrip("/") + "/"):
    refuse(f"{p} is under {r} (case-insensitive)")
env = {k: v for k, v in os.environ.items() if k not in ("GIT_DIR", "GIT_WORK_TREE", "GIT_CEILING_DIRECTORIES", "GIT_DISCOVERY_ACROSS_FILESYSTEM")}
try:
    g = subprocess.run(["git", "-C", anc, "rev-parse", "--show-toplevel"], capture_output=True, text=True, env=env)
    if g.returncode == 0:
        refuse(f"{anc} is in the git work tree {g.stdout.strip()}")
    if "not a git repository" not in g.stderr:
        g2 = subprocess.run(["git", "-C", anc, "rev-parse", "--git-dir"], capture_output=True, text=True, env=env)
        if g2.returncode == 0:
            refuse(f"{anc} is inside a git directory {g2.stdout.strip()}")
except FileNotFoundError:
    pass  # no git on this machine: the inode and case checks above stand
print(p)
PY
)" || { echo "$out" >&2; exit 2; }
  printf '%s\n' "$out"
}
