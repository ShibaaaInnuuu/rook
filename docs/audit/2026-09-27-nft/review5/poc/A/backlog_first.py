#!/usr/bin/env python3
"""Review round 5, part A (A5-L-2's fix): the three delivery orders, day by day (a 24 h window per batch, as the H3 limiter counts it).
- deadline-first, worst order: the order tickets resolved in, Azuki's last (what ethflow::eth_act does: `sort_by_key(deliver_by)`);
- backlog-first: each delivery taken from the collection with the most items still waiting (whose own day-limit is not used up);
- the robust check for 2 usable days: day 1 is never global-bound (sum of min(items, per) <= global) and day 2's rest fits.
Every pair here passes the canister's current rule (each collection alone, and the total)."""
def run(held, per, glob, days, pick):
    rem = dict(held); out_by_day = []
    for _ in range(days):
        sent = {k: 0 for k in rem}; n = 0
        while n < glob:
            ok = [k for k in rem if rem[k] > 0 and sent[k] < per]
            if not ok: break
            k = pick(ok, rem); rem[k] -= 1; sent[k] += 1; n += 1
        out_by_day.append(n)
    return {k: v for k, v in rem.items() if v}
order = ["milady", "mayc", "pudgy", "bayc", "azuki"]  # Azuki's tickets resolved last
deadline_worst = lambda ok, rem: min(ok, key=order.index)
backlog_first = lambda ok, rem: max(ok, key=lambda k: (rem[k], -order.index(k)))
robust = lambda held, per, glob: sum(min(n, per) for n in held.values()) <= glob and sum(max(0, n - per) for n in held.values()) <= glob and all(n <= 2 * per for n in held.values())
cfgs = [("(a) install 13/17/13/3/2 @ 12/24", dict(milady=13, azuki=17, mayc=13, pudgy=3, bayc=2), 12, 24),
        ("(c) interim 13/17/13/3/2 @ 12/31", dict(milady=13, azuki=17, mayc=13, pudgy=3, bayc=2), 12, 31),
        ("(c) end J7 16/22/17/4/2 @ 12/31", dict(milady=16, azuki=22, mayc=17, pudgy=4, bayc=2), 12, 31),
        ("J7 @ 22/31", dict(milady=16, azuki=22, mayc=17, pudgy=4, bayc=2), 22, 31),
        ("J7 @ 12/42", dict(milady=16, azuki=22, mayc=17, pudgy=4, bayc=2), 12, 42),
        ("(b) 16/22/10 held @ 12/24", dict(milady=16, azuki=22, mayc=10), 12, 24)]
for name, held, per, glob in cfgs:
    print(f"{name}: deadline-first worst order leaves {run(held, per, glob, 2, deadline_worst) or 'nothing'}; "
          f"backlog-first leaves {run(held, per, glob, 2, backlog_first) or 'nothing'}; robust check: {'passes' if robust(held, per, glob) else 'refuses'}")
