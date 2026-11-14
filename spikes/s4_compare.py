"""Summarise the s4 runs: cost table, and how much T moves between settings (noise check).

Run: .venv/Scripts/python s4_compare.py
"""
import json
from pathlib import Path

import numpy as np

import sys
R = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent / "s4_results"
runs = {p.stem: json.loads(p.read_text()) for p in sorted(R.glob("k*.json"))}

print(f"{'run':18s} {'plies':>5s} {'min':>6s} {'s/ply':>6s} {'pos/ply':>8s} {'max':>5s} "
      f"{'SF req':>7s} {'SF run':>7s} {'hit%':>5s} {'Maia run':>8s}")
for name, r in runs.items():
    s = r["summary"]
    hit = 100 * (1 - s["sf_searches_run"] / s["sf_requests"])
    print(f"{name:18s} {s['plies']:5d} {s['seconds']/60:6.1f} {s['sec_per_ply']:6.2f} "
          f"{s['tree_positions_per_ply_mean']:8.1f} {s['tree_positions_per_ply_max']:5d} "
          f"{s['sf_requests']:7d} {s['sf_searches_run']:7d} {hit:5.1f} {s['maia_positions_run']:8d}")


def series(name, key):
    return np.array([row[key] for row in runs[name]["rows"]])


def cmp(a, b, label):
    if a not in runs or b not in runs:
        return
    for key in ("T_white", "T_black", "v_star_white"):
        x, y = series(a, key), series(b, key)
        n = min(len(x), len(y))
        x, y = x[:n], y[:n]
        print(f"  {label:28s} {key:13s} mean|diff|={np.mean(np.abs(x-y)):.3f}  "
              f"max|diff|={np.max(np.abs(x-y)):.3f}  corr={np.corrcoef(x, y)[0,1]:.2f}")


print("\nhow much T changes between settings (same 80-ply game):")
cmp("k2_p0.9_n50000", "k2_p0.9_n200000", "nodes 50k vs 200k (k=2)")
cmp("k1_p0.9_n50000", "k2_p0.9_n50000", "k=1 vs k=2")
cmp("k2_p0.9_n50000", "k3_p0.9_n50000", "k=2 vs k=3")
cmp("k2_p0.8_n50000", "k2_p0.9_n50000", "p=0.8 vs p=0.9")
cmp("k2_p0.9_n50000", "k2_p0.95_n50000", "p=0.9 vs p=0.95")

print("\nT distribution per run (both sides pooled):")
for name in runs:
    t = np.concatenate([series(name, "T_white"), series(name, "T_black")])
    print(f"  {name:18s} mean={t.mean():.3f} median={np.median(t):.3f} p90={np.quantile(t,0.9):.3f} "
          f"min={t.min():.3f} share<0={np.mean(t<0):.2f}")
