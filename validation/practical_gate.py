"""Phase 4 gate: does the practical best move point at the trap at low ratings, and stop at
high ratings? Runs `trickiness practical` for every trap position and rating, saves the JSON,
and prints a table.

    spikes/.venv/Scripts/python validation/practical_gate.py [--ratings 800,1100,1500,1900,2300]
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "validation" / "results" / "practical"
EXE = ROOT / "target" / "release" / "trickiness.exe"

ap = argparse.ArgumentParser()
ap.add_argument("--ratings", default="800,1100,1500,1900,2300")
ap.add_argument("--extra", default="", help="extra CLI flags, e.g. --lambda 0.3 --tau 0.7")
ap.add_argument("--tag", default="", help="name for this variant's results folder and report")
args = ap.parse_args()
if args.tag:
    OUT = OUT.with_name(f"practical_{args.tag}")
ratings = [int(r) for r in args.ratings.split(",")]
OUT.mkdir(parents=True, exist_ok=True)
traps = yaml.safe_load((ROOT / "validation" / "traps.yaml").read_text())

table = {}
for t in traps:
    for r in ratings:
        path = OUT / f"{t['id']}_{r}.json"
        if not path.exists():
            cmd = [str(EXE), "practical", "--moves", t["moves"], "--elo", str(r), "--quiet"] + args.extra.split()
            res = subprocess.run(cmd, capture_output=True, text=True, cwd=ROOT)
            if res.returncode != 0:
                print(t["id"], r, "FAILED:", res.stderr[-300:], file=sys.stderr)
                continue
            path.write_text(res.stdout, encoding="utf-8")
        d = json.loads(path.read_text(encoding="utf-8"))
        c = d["candidates"]
        pb, ob = c[d["practical_best"]], c[d["objective_best"]]
        trap = next((x for x in c if x["san"].rstrip("+#") == t["trap"]), None)
        rank = sorted(c, key=lambda x: -x["practical"]).index(trap) + 1 if trap else None
        others = [x["practical"] for x in c if x is not trap]
        table[(t["id"], r)] = dict(pb=pb["san"], ob=ob["san"], trap_rank=rank, second_q=max(others) if others else 0.0,
                                   trap_q=trap["practical"] if trap else None, trap_obj=trap["objective"] if trap else None,
                                   best_q=pb["practical"], miss=trap["miss_chance"] if trap else None)
        print(f"{t['id']:20s} {r:5d}  practical best {pb['san']:6s} objective best {ob['san']:6s}  "
              f"trap {t['trap']:6s} rank {rank}  Q {table[(t['id'], r)]['trap_q'] or 0:.3f} vs best {pb['practical']:.3f}", flush=True)

# Restated gate (SPEC.md Phase 4, DECISIONS 4.8): the trap move's practical margin over the
# best other candidate falls as the opponent's rating rises. Pass = lower at the highest
# rating than at the lowest, and no step up by more than TOL (engine noise at 50k nodes).
TOL = 0.005
lines = ["Margin = practical value of the trap move minus the best other candidate (positive: the trap is the practical best).", "",
         "| Trap (move) | " + " | ".join(str(r) for r in ratings) + " | Trend |", "|" + " --- |" * (len(ratings) + 2)]
passed = []
for t in traps:
    mg = []
    for r in ratings:
        e = table.get((t["id"], r))
        mg.append(None if not e or e["trap_rank"] is None else e["trap_q"] - (e["second_q"] if e["trap_rank"] == 1 else e["best_q"]))
    if any(v is None for v in mg):
        verdict = "fail (not always a candidate)"
    else:
        steps_ok = all(b <= a + TOL for a, b in zip(mg, mg[1:]))
        verdict = "**pass**" if mg[-1] < mg[0] and steps_ok else "fail"
        if verdict == "**pass**":
            passed.append(t["id"])
    cells = ["–" if v is None else (f"**{v:+.3f}**" if v > 0 else f"{v:+.3f}") for v in mg]
    lines.append(f"| {t['id']} ({t['trap']}) | " + " | ".join(cells) + f" | {verdict} |")
strict = [t["id"] for t in traps
          if any(table.get((t["id"], r), {}).get("trap_rank") == 1 for r in ratings[: len(ratings) // 2])
          and all(table.get((t["id"], r), {}).get("trap_rank") != 1 for r in ratings[-2:])]
lines += ["", f"Gate (restated): the margin falls as rating rises in **{len(passed)}/{len(traps)}** traps: {', '.join(passed) or 'none'}",
          f"For reference, the original strict test (trap is the top move at low ratings, not at high): {len(strict)}/{len(traps)}: {', '.join(strict) or 'none'}"]
text = "\n".join(lines)
name = f"practical_gate_{args.tag}.md" if args.tag else "practical_gate.md"
(ROOT / "validation" / "results" / name).write_text(
    f"Variant: {args.tag or 'spec (Stockfish leaves)'} {args.extra}\n\n" + text + "\n", encoding="utf-8")
print("\n" + text)
