#!/usr/bin/env python3
"""Review round 5, part A (A3-L-3 re-attack): golive-check monglobal's placeholder rule, the exact expression of
script/golive-check.sh (round 4: '<' anywhere, or a TODO-style word), on values an operator might leave in the monitor file."""
import re
rule = lambda v: "<" in v or bool(re.search(r"\b(TODO|TBD|FIXME|CHANGEME|CHANGE_ME|XXX+|REPLACE_?ME)\b", v, re.I))
cases = ["https://<runner host>/healthz", "https://TODO/healthz", "https://todo.example/healthz",
         "https://TODO_RUNNER/healthz", "https://RUNNER_TODO/healthz", "https://runner-TODO.up.railway.app/healthz",
         "https://YOUR_RUNNER_HOST/healthz", "https://your-runner.example.com/healthz", "https://PLACEHOLDER/healthz",
         "https://runner.example.com/healthz", "0x0000000000000000000000000000000000000000", "https://FILL_ME_IN/healthz",
         "https://xxx.example/healthz", "https://[runner]/healthz", "https://{{RUNNER}}/healthz", "https://runner-host/healthz"]
for v in cases:
    print(("refused  " if rule(v) else "PASSES   ") + v)
