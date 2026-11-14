"""Phase 2, steps 2-3: calibration of T and D on Lichess games, and parameter tuning.

T test: for each sampled position and each player X, predicted T_X (per (k, p) setting) vs X's
realized loss = sum over X's next k moves of v*_X(before the move) - v*_X(after), with v*
from Stockfish (same settings as the core). Positions are bucketed by predicted T.

D test: Maia's D at ply 30 (after move 15) vs whether the game was drawn, bucketed by D.
Stockfish's own draw probability is shown alongside for comparison.

Tuning: (k, p) by calibration error of T; ε and δ by how well stable_moves and trap_mass
rank X's realized loss on the next move.

    spikes/.venv/Scripts/python validation/calibrate.py
"""
import json
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np

R = Path(__file__).resolve().parent / "results"
PLOTS = R / "plots"
PLOTS.mkdir(exist_ok=True)
BANDS = {"1400_1600": ("1400–1600", "#2a78d6"), "1900_2100": ("1900–2100", "#eb6834")}
SURFACE, INK, INK2, GRID = "#fcfcfb", "#0b0b0b", "#52514e", "#e4e3df"
SETTINGS = ["k1_p0.8", "k1_p0.9", "k1_p0.95", "k2_p0.8", "k2_p0.9", "k2_p0.95"]
N_BINS = 8

plt.rcParams.update({
    "figure.facecolor": SURFACE, "axes.facecolor": SURFACE, "savefig.facecolor": SURFACE,
    "axes.edgecolor": GRID, "axes.labelcolor": INK2, "xtick.color": INK2, "ytick.color": INK2,
    "text.color": INK, "axes.grid": True, "grid.color": GRID, "grid.linewidth": 0.8,
    "axes.spines.top": False, "axes.spines.right": False, "font.size": 10,
})


def load(path):
    return [json.loads(l) for l in open(path, encoding="utf-8")] if path.exists() else []


def spearman(a, b):
    ra = np.argsort(np.argsort(a)).astype(float)
    rb = np.argsort(np.argsort(b)).astype(float)
    return float(np.corrcoef(ra, rb)[0, 1])


def realized_loss(row, side, k):
    """Sum of X's per-move loss over its next k moves (fewer if the game ends)."""
    path = row["v_star_path_white"]
    t, stm = row["t"], row["side_to_move"]
    vx = [v if side == "white" else 1 - v for v in path]
    first = 0 if side == stm else 1
    loss, n = 0.0, 0
    for i in range(first, len(vx) - 1, 2):
        if n == k:
            break
        loss += vx[i] - vx[i + 1]
        n += 1
    return loss if n else None


def pairs(rows, setting):
    k = int(setting[1])
    pred, real = [], []
    for r in rows:
        for i, side in enumerate(("white", "black")):
            rl = realized_loss(r, side, k)
            if rl is not None:
                pred.append(r["T"][setting][i])
                real.append(rl)
    return np.array(pred), np.array(real)


def bins(pred, real, n=N_BINS):
    """Quantile bins of predicted T: (mean pred, mean real, SE real, count)."""
    order = np.argsort(pred)
    out = []
    for idx in np.array_split(order, n):
        if len(idx) == 0:
            continue
        out.append((pred[idx].mean(), real[idx].mean(), real[idx].std(ddof=1) / np.sqrt(len(idx)), len(idx)))
    return np.array(out)


def auc(score, label):
    """Probability a random positive outranks a random negative (ties count half)."""
    pos, neg = score[label == 1], score[label == 0]
    if len(pos) == 0 or len(neg) == 0:
        return float("nan")
    allv = np.concatenate([pos, neg])
    ranks = np.argsort(np.argsort(allv)).astype(float) + 1
    # average ranks for ties
    _, inv, counts = np.unique(allv, return_inverse=True, return_counts=True)
    sums = np.bincount(inv, weights=ranks)
    ranks = (sums / counts)[inv]
    return float((ranks[: len(pos)].sum() - len(pos) * (len(pos) + 1) / 2) / (len(pos) * len(neg)))


def calib_error(b):
    return float(np.average(np.abs(b[:, 0] - b[:, 1]), weights=b[:, 3]))


report = []
t_data = {b: load(R / f"t_{b}.jsonl") for b in BANDS}
d_data = {b: load(R / f"d_{b}.jsonl") for b in BANDS}

# ------------------------------------------------------------------ T: grid table
report.append("## T calibration\n")
report.append("Realized loss = X's own expected-score loss over its next k moves (Stockfish v* before minus after each move).\n")
report.append("| Band | Setting | Pairs | Mean T | Mean realized | Calibration error | Relative error | Slope (realized ~ T) | Spearman |")
report.append("| --- | --- | --- | --- | --- | --- | --- | --- | --- |")
scores = {}
for b, rows in t_data.items():
    if not rows:
        continue
    for s in SETTINGS:
        pred, real = pairs(rows, s)
        bb = bins(pred, real)
        slope = float(np.polyfit(pred, real, 1)[0])
        ce = calib_error(bb)
        rho = spearman(pred, real)
        rel = ce / real.mean()
        scores.setdefault(s, []).append((rel, rho, slope))
        report.append(f"| {BANDS[b][0]} | {s} | {len(pred)} | {pred.mean():.3f} | {real.mean():.3f} | {ce:.3f} | {rel:.2f} | {slope:.2f} | {rho:.2f} |")
best = min(scores, key=lambda s: np.mean([c for c, _, _ in scores[s]])) if scores else None

# Bootstrap over games: how sure is the ranking of settings?
rng = np.random.default_rng(0)
boot = {s: [] for s in SETTINGS}
for _ in range(300):
    for s in SETTINGS:
        rels = []
        for b, rows in t_data.items():
            if not rows:
                continue
            idx = rng.integers(0, len(rows), len(rows))
            pr, re_ = pairs([rows[i] for i in idx], s)
            rels.append(calib_error(bins(pr, re_)) / re_.mean())
        boot[s].append(np.mean(rels))
report.append("\n| Setting | Relative error, 90% bootstrap interval (mean over bands) |\n| --- | --- |")
report += [f"| {s} | {np.quantile(v, 0.05):.2f} – {np.quantile(v, 0.95):.2f} |" for s, v in boot.items()]
report.append("\nRelative error = calibration error / mean realized loss, so k = 1 (one move) and k = 2 (two moves) compare fairly.")
report.append(f"Best (lowest mean relative error across bands): **{best}**\n")

# ------------------------------------------------------------------ T: plots
if best:
    fig, axes = plt.subplots(1, 2, figsize=(10, 4.4), sharey=True)
    for ax, s in zip(axes, [best, "k2_p0.9" if best != "k2_p0.9" else "k1_p0.9"]):
        top = 0.05
        for b, rows in t_data.items():
            if not rows:
                continue
            label, color = BANDS[b]
            pred, real = pairs(rows, s)
            bb = bins(pred, real)
            ax.errorbar(bb[:, 0], bb[:, 1], yerr=1.96 * bb[:, 2], color=color, lw=2, marker="o", ms=6,
                        capsize=0, elinewidth=1.2, label=f"{label} (n={len(pred)})",
                        markeredgecolor=SURFACE, markeredgewidth=1.5)
            top = max(top, bb[:, 0].max(), (bb[:, 1] + 1.96 * bb[:, 2]).max())
        lim = top * 1.08
        ax.plot([0, lim], [0, lim], ls="--", lw=1, color=INK2, label="perfect calibration")
        ax.set_xlim(min(0, ax.get_xlim()[0]), lim)
        ax.set_title(f"{s.replace('_', ', ').replace('k', 'k = ').replace('p', 'p = ')}", color=INK, loc="left", fontsize=11)
        ax.set_xlabel("Predicted T (bucket mean)")
    axes[0].set_ylabel("Realized loss over next k moves (mean, 95% CI)")
    axes[0].legend(frameon=False, loc="upper left")
    fig.suptitle("T calibration on Lichess games (Oct 2026), buckets of equal size", x=0.06, ha="left", color=INK)
    fig.tight_layout()
    fig.savefig(PLOTS / "t_calibration.png", dpi=150)
    plt.close(fig)

# ------------------------------------------------------------------ ε, δ tuning
report.append("## Diagnostics tuning (ε, δ)\n")
report.append("Positions where X is to move; target = X's realized loss on that one move. "
              "Spearman with trap mass should be positive, with stable_moves negative.\n")
p_for_trap = float(best.split("_p")[1]) if best else 0.9
eps_grid = [0.0025, 0.005, 0.01, 0.02, 0.03, 0.05, 0.08]
delta_grid = [0.01, 0.02, 0.03, 0.05, 0.10, 0.15, 0.20, 0.30]
eps_rho = {e: [] for e in eps_grid}
delta_rho = {d: [] for d in delta_grid}
for b, rows in t_data.items():
    recs = []
    for r in rows:
        for dec in r["decisions"] or []:
            if dec["offset"] != 0:
                continue
            rl = realized_loss(r, dec["side"], 1)
            if rl is not None:
                recs.append((dec, rl))
    if not recs:
        continue
    real = np.array([rl for _, rl in recs])
    for e in eps_grid:
        st = np.array([sum(s >= max(d["mpv_scores"]) - e for s in d["mpv_scores"]) for d, _ in recs])
        eps_rho[e].append(spearman(st, real))
    for dl in delta_grid:
        tm = []
        for d, _ in recs:
            cum, m = 0.0, 0.0
            for p, v in d["moves"]:
                if v < d["v_best"] - dl:
                    m += p
                cum += p
                if cum >= p_for_trap - 1e-9:
                    break
            tm.append(m)
        delta_rho[dl].append(spearman(np.array(tm), real))
report.append("| ε | Spearman(stable_moves, loss) |\n| --- | --- |")
report += [f"| {e} | {np.mean(v):+.3f} |" for e, v in eps_rho.items() if v]
report.append("\n| δ | Spearman(trap_mass, loss) |\n| --- | --- |")
report += [f"| {d} | {np.mean(v):+.3f} |" for d, v in delta_rho.items() if v]
if any(eps_rho.values()):
    be = min(eps_rho, key=lambda e: np.mean(eps_rho[e]))
    bd = max(delta_rho, key=lambda d: np.mean(delta_rho[d]))
    report.append(f"\nBest: ε = **{be}**, δ = **{bd}**\n")

# ------------------------------------------------------------------ D test
report.append("## D calibration (ply 30, after move 15)\n")
report.append("| Band | Games | Draw rate | Brier: constant | Brier: Maia D | Brier: Stockfish D | AUC: Maia D | AUC: Stockfish D |")
report.append("| --- | --- | --- | --- | --- | --- | --- | --- |")
fig, ax = plt.subplots(figsize=(5.6, 4.6))
top = 0.05
for b, rows in d_data.items():
    if not rows:
        continue
    label, color = BANDS[b]
    drawn = np.array([r["result"] == "1/2-1/2" for r in rows], float)
    dm = np.array([r["maia_wdl"][1] for r in rows])
    ds = np.array([r["sf_wdl"][1] for r in rows])
    base = drawn.mean()
    brier = lambda q: float(np.mean((q - drawn) ** 2))
    report.append(f"| {BANDS[b][0]} | {len(rows)} | {base:.3f} | {brier(np.full_like(dm, base)):.4f} | "
                  f"{brier(dm):.4f} | {brier(ds):.4f} | {auc(dm, drawn):.2f} | {auc(ds, drawn):.2f} |")
    bb = bins(dm, drawn, 10)
    ax.errorbar(bb[:, 0], bb[:, 1], yerr=1.96 * bb[:, 2], color=color, lw=2, marker="o", ms=6, capsize=0,
                elinewidth=1.2, label=f"{label} (n={len(rows)})", markeredgecolor=SURFACE, markeredgewidth=1.5)
    top = max(top, bb[:, 0].max(), (bb[:, 1] + 1.96 * bb[:, 2]).max())
lim = top * 1.08
ax.plot([0, lim], [0, lim], ls="--", lw=1, color=INK2, label="perfect calibration")
ax.set_xlim(0, lim)
ax.set_ylim(0, lim)
ax.set_xlabel("Maia D at move 15 (decile mean)")
ax.set_ylabel("Share of games drawn (95% CI)")
ax.set_title("D calibration: predicted vs actual draws", loc="left", fontsize=11, color=INK)
ax.legend(frameon=False, loc="upper left")
fig.tight_layout()
fig.savefig(PLOTS / "d_calibration.png", dpi=150)
plt.close(fig)

text = "\n".join(report)
(R / "calibration.md").write_text(text + "\n", encoding="utf-8")
print(text.encode("ascii", "replace").decode())
