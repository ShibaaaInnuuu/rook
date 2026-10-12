#!/usr/bin/env python3
"""Review round 5, part A (A4-L-1): golive-check's EXIT_PY (extracted unchanged from script/golive-check.sh) against the canister's
validate_custody on the same grid (the GRID lines printed by r5a_exit_rule_append_to_canister_rs.rs `grid_for_the_python_side`).

usage: exit_agreement.py <exit_py.py> <grid.txt>
A disagreement is printed when golive-check's verdict differs from the canister's exit-rule verdict. Rows the canister refuses
before the exit rule (`other:`: a malformed policy or limits) are counted apart: golive-check does not re-validate those, and does
not need to (the canister refuses them), but they show where the two differ in what they accept.
"""
import json, subprocess, sys

exit_py, grid = sys.argv[1], sys.argv[2]
code = open(exit_py).read()
agree = differ = other = 0
rows = []
for line in open(grid):
    _, items, gcap, per, glob, verdict = line.split()
    items = [int(x) for x in items.split(",")]
    src = {"items": {f"c{i}": n for i, n in enumerate(items)}, "global_cap": None if gcap == "-" else int(gcap),
           "per": int(per), "global": int(glob)}
    out = subprocess.run([sys.executable, "-c", code, "t", json.dumps(src)], capture_output=True, text=True)
    g = "fail" if "FAIL" in out.stdout else ("ok" if out.stdout.startswith("ok") else ("crash:" + out.stderr.strip().splitlines()[-1] if out.stderr else "silent"))
    if verdict.startswith("other:"):
        other += 1
        rows.append(f"canister refuses before the exit rule ({verdict[6:]}); golive-check exit read: {g}  <- {line.strip()}")
        continue
    c = "ok" if verdict == "ok" else "fail"
    if c == g:
        agree += 1
    else:
        differ += 1
        rows.append(f"DISAGREE canister={verdict} golive={g}  <- {line.strip()}")
for r in rows:
    print(r)
print(f"\nexit-rule rows: {agree} agree, {differ} disagree; {other} rows refused by the canister before the exit rule")
