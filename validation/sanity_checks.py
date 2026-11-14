"""Phase 2, step 1: ordering checks on the sanity positions (SPEC.md Purpose table).

Reads validation/results/sanity.json (from `validate sanity`), prints the table and the checks,
writes validation/results/sanity.md, and exits 1 if any check fails.

    spikes/.venv/Scripts/python validation/sanity_checks.py [--k 2]
"""
import argparse
import json
import sys
from collections import defaultdict
from pathlib import Path

ap = argparse.ArgumentParser()
ap.add_argument("--k", type=int, default=2)
ap.add_argument("--near-zero", type=float, default=0.05, help="'close to 0' threshold")
args = ap.parse_args()

R = Path(__file__).resolve().parent / "results"
data = json.loads((R / "sanity.json").read_text())
rows = [r for r in data["rows"] if r["k"] == args.k]
ratings = sorted({r["rating"] for r in rows})
mean = lambda xs: sum(xs) / len(xs) if xs else float("nan")


def t_of(r, side):
    return r["t_white"] if side == "white" else r["t_black"]


by = defaultdict(dict)  # id -> rating -> row
for r in rows:
    by[r["id"]][r["rating"]] = r

lines = [f"Settings: k={args.k}, p={rows[0]['p']}, {data['nodes']} nodes, {data['stockfish']}, {data['maia']}", ""]
hdr = "| Position | Category | v* (W) | " + " | ".join(f"T_W {int(x)}" for x in ratings) + " | " + \
      " | ".join(f"T_B {int(x)}" for x in ratings) + " | " + " | ".join(f"D {int(x)}" for x in ratings) + " |"
lines += [hdr, "|" + " --- |" * (3 + 3 * len(ratings))]
for pid, rr in by.items():
    first = next(iter(rr.values()))
    lines.append(f"| {pid} | {first['category']} | {first['v_star_white']:.2f} | "
                 + " | ".join(f"{rr[x]['t_white']:+.3f}" for x in ratings) + " | "
                 + " | ".join(f"{rr[x]['t_black']:+.3f}" for x in ratings) + " | "
                 + " | ".join(f"{rr[x]['draw_rate']:.3f}" for x in ratings) + " |")

checks = []
rb_def = {}


def check(name, ok, detail):
    checks.append((name, ok, detail))


def cat_rows(cat, rating):
    return [r for r in rows if r["category"] == cat and r["rating"] == rating]


for x in ratings:
    rr = cat_rows("rook_vs_rook", x)
    bp = cat_rows("blocked_pawns", x)
    sh = cat_rows("sharp_opening", x)
    rb = cat_rows("rb_vs_r", x)
    tr = cat_rows("opening_trap", x)
    rr_t = [t_of(r, s) for r in rr for s in ("white", "black")]
    bp_t = [t_of(r, s) for r in bp for s in ("white", "black")]
    sh_t = [t_of(r, s) for r in sh for s in ("white", "black")]
    att = [t_of(r, r["roles"]["attacker"]) for r in rb]
    dfn = [t_of(r, r["roles"]["defender"]) for r in rb]
    vic = [t_of(r, r["roles"]["victim"]) for r in tr]
    trp = [t_of(r, r["roles"]["trapper"]) for r in tr]
    n = args.near_zero
    check(f"{int(x)}: rook vs rook, both T close to 0", max(rr_t) < n, f"max {max(rr_t):+.3f}")
    check(f"{int(x)}: blocked pawns, both T close to 0", max(bp_t) < n, f"max {max(bp_t):+.3f}")
    check(f"{int(x)}: sharp opening, T medium (above rook vs rook)", mean(sh_t) > mean(rr_t) + n,
          f"mean {mean(sh_t):.3f} vs {mean(rr_t):.3f}")
    check(f"{int(x)}: R+B vs R, attacker T close to 0", max(att) < n, f"max {max(att):+.3f}")
    # SPEC.md (revised, DECISIONS 2.9): above the attacker's, not "high"; T covers only k moves.
    check(f"{int(x)}: R+B vs R, defender T above attacker T", mean(dfn) > mean(att),
          f"defender {mean(dfn):.3f} vs attacker {mean(att):.3f}")
    rb_def[x] = mean(dfn)
    check(f"{int(x)}: trap, victim T > trapper T in every trap", all(v > t for v, t in zip(vic, trp)),
          " ".join(f"{r['id'].removeprefix('trap_')}:{v:.2f}/{t:.2f}" for r, v, t in zip(tr, vic, trp)))
    rr_d = mean([r["draw_rate"] for r in rr])
    sh_d = mean([r["draw_rate"] for r in sh])
    check(f"{int(x)}: D(rook vs rook) > D(sharp opening)", rr_d > sh_d, f"{rr_d:.3f} > {sh_d:.3f}")

check("R+B vs R, defender T falls as rating rises",
      all(rb_def[a] > rb_def[b] for a, b in zip(ratings, ratings[1:])),
      " > ".join(f"{rb_def[x]:.3f}" for x in ratings))

# T falls as rating rises: every (position, side) whose T is meaningful at the lowest rating.
pairs = [(pid, s) for pid in by for s in ("white", "black") if t_of(by[pid][ratings[0]], s) > args.near_zero]
falls = [(pid, s) for pid, s in pairs if t_of(by[pid][ratings[-1]], s) < t_of(by[pid][ratings[0]], s)]
means = [mean([t_of(by[pid][x], s) for pid, s in pairs]) for x in ratings]
check("T falls as rating rises (mean over positions with T > near-zero)",
      all(a > b for a, b in zip(means, means[1:])), " > ".join(f"{m:.3f}" for m in means))
check("T falls as rating rises (each position-side, lowest vs highest rating)",
      len(falls) == len(pairs), f"{len(falls)}/{len(pairs)} fall")

lines += ["", "| Check | Result | Detail |", "| --- | --- | --- |"]
lines += [f"| {n} | {'PASS' if ok else '**FAIL**'} | {d} |" for n, ok, d in checks]
text = "\n".join(lines)
print(text)
(R / f"sanity_k{args.k}.md").write_text(text + "\n", encoding="utf-8")
failed = sum(not ok for _, ok, _ in checks)
print(f"\n{len(checks) - failed}/{len(checks)} checks pass")
sys.exit(1 if failed else 0)
