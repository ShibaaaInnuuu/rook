#!/usr/bin/env bash
# Review round 5, part A: fixes-round4 §6 option (b) (a global custody cap of 48 over J7's per-collection caps) through golive-check's
# own reads. Extracted UNCHANGED from script/golive-check.sh of a private copy (COPY): the J7 "no total binds" read of custody_caps()
# (capsread's inline Python) and the A4-L-1 exit read (EXIT_PY). The canister ACCEPTS (b) (r5a option_c_sequence...: "install (b)").
set -u
COPY=${COPY:-/tmp/rook-r5/A}
G="$COPY/script/golive-check.sh"
awk '/local tot; tot=\$\(printf/{f=1;next} f&&/^print\("ok" if g\[0\]/{print; exit} f' "$G" | sed -e "s/')\$//" > "$COPY/j7_total.py"
awk '/^read -r -d .. EXIT_PY <</{f=1;next} /^PY$/{if(f){exit}} f' "$G" > "$COPY/exit_py.py"
E=15000000000000000000
cols='"1:0x5af0d9827e0c53e4799bb226655a1de152a425a5": [16, '$E', '$E'], "1:0xed5af388653567af2f388e6224dc7c4b3241c544": [22, '$E', '$E'], "1:0x60e4d786628fea6478f785a6d7e704777c86a7c6": [17, '$E', '$E'], "1:0xbd3531da5cf5857e7cfaa92426877b022e612cf8": [4, '$E', '$E'], "1:0xbc4ca0eda7647a8ab7c2061c2e118a18a936f13d": [2, '$E', '$E']'
for g in 61 48; do
  CAPS='{"global": ['$g', 75000000000000000000], "cols": {'"$cols"'}}'
  echo "--- the canister's custody_caps(): J7's per-collection caps, global_max_items $g"
  echo "    J7 total read (capsread): $(printf '%s' "$CAPS" | python3 "$COPY/j7_total.py")"
  j=$(printf '%s' "$CAPS" | python3 -c 'import json,sys; c=json.load(sys.stdin); print(json.dumps({"items": {k: v[0] for k, v in c["cols"].items()}, "global_cap": c["global"][0], "per": 12, "global": 24}))')
  echo "    exit read (canexit, 12/24): $(python3 "$COPY/exit_py.py" "the canister's pair" "$j" | cut -c1-160)"
done
