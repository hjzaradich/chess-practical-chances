"""Phase 4, option A: a rating-calibrated leaf model (DECISIONS 4.5).

Maps Stockfish's evaluation at a position (50k nodes, the core's settings) to the outcome of a
*human* game between players of given ratings: P(win), P(draw), P(loss) for the side to move.

Ordered logit, symmetric between the two sides:
    x   = Stockfish's eval in pawns, side to move (±10 cap, mate ±12; same rule as the core)
    x~  = s*tanh(x/s) for a saturation scale s chosen on validation games (s = 0: x as is)
    z   = (mean rating - 1500) / 400,   dr = (rating to move - other rating) / 400
    m   = (material on the board - 40) / 40                  (only if validation prefers it)
    eta = (a0 + a1*z + a2*m) * x~ + g*dr
    h   = exp(w0 + w1*z + w2*m)                               (half-width of the draw band)
    P(win) = sigmoid(eta - h),  P(loss) = sigmoid(-eta - h),  P(draw) = the rest

Games are split by a hash of their ID: 60% fit / 10% validation (choose s and the material
term), then refit on those 70%; the other 30% is used only for the final report.
Writes validation/results/calib/leaf_model.json, leaf_model.md and plots/leaf_calibration.png.

    spikes/.venv/Scripts/python validation/fit_leaf_model.py
"""
import hashlib
import json
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np
import torch

R = Path(__file__).resolve().parent / "results" / "calib"
PLOTS = R.parent / "plots"
PLOTS.mkdir(exist_ok=True)
SURFACE, INK, INK2, GRID = "#fcfcfb", "#0b0b0b", "#52514e", "#e4e3df"
GROUPS = [("800–1399", 800, 1400, "#2a78d6"), ("1400–1999", 1400, 2000, "#eb6834"), ("2000–2399", 2000, 2400, "#1baf7a")]
plt.rcParams.update({
    "figure.facecolor": SURFACE, "axes.facecolor": SURFACE, "savefig.facecolor": SURFACE,
    "axes.edgecolor": GRID, "axes.labelcolor": INK2, "xtick.color": INK2, "ytick.color": INK2,
    "text.color": INK, "axes.grid": True, "grid.color": GRID, "grid.linewidth": 0.8,
    "axes.spines.top": False, "axes.spines.right": False, "font.size": 10,
})

# ---------------------------------------------------------------- data
rows = []
for f in sorted(R.glob("evals_*.jsonl")):
    for line in open(f, encoding="utf-8"):
        r = json.loads(line)
        if r["result"] not in ("1-0", "0-1", "1/2-1/2") or not r["white_elo"] or not r["black_elo"]:
            continue
        if r.get("cp") is None and r.get("mate") is None:
            continue
        rows.append(r)
print(len(rows), "positions")


def outcome(r):  # for the side to move: 0 loss, 1 draw, 2 win
    if r["result"] == "1/2-1/2":
        return 1
    return 2 if (r["result"] == "1-0") == (r["stm"] == "white") else 0


def pawns(r):  # same rule as EngineLine::pawns in the core
    if r.get("mate") is not None:
        return 12.0 if r["mate"] > 0 else -12.0
    return float(np.clip(r["cp"] / 100.0, -10, 10))


W, D, L = (np.array([r["sf_wdl"][i] for r in rows]) for i in range(3))
x_raw = np.array([pawns(r) for r in rows])
r_stm = np.array([r["white_elo"] if r["stm"] == "white" else r["black_elo"] for r in rows], float)
r_oth = np.array([r["black_elo"] if r["stm"] == "white" else r["white_elo"] for r in rows], float)
mean_r = (r_stm + r_oth) / 2
z = (mean_r - 1500) / 400
dr = (r_stm - r_oth) / 400
m = (np.array([r["material"] for r in rows], float) - 40) / 40
y = np.array([outcome(r) for r in rows])
hsh = np.array([int(hashlib.sha1(r["site"].encode()).hexdigest(), 16) % 10 for r in rows])
test, val, fit_set = hsh < 3, hsh == 3, hsh >= 4
train = ~test
print(f"fit {fit_set.sum()}  validation {val.sum()}  test {test.sum()}")

T = lambda a: torch.tensor(a, dtype=torch.float64)
Z, DR, M, Y = T(z), T(dr), T(m), torch.tensor(y)


def xt(scale):
    return scale * np.tanh(x_raw / scale) if scale > 0 else x_raw


def probs(p, X, idx, phase):
    a = p["a0"] + p["a1"] * Z[idx] + (p["a2"] * M[idx] if phase else 0)
    eta = a * X[idx] + p["g"] * DR[idx]
    h = torch.exp(p["w0"] + p["w1"] * Z[idx] + (p["w2"] * M[idx] if phase else 0))
    pw = torch.sigmoid(eta - h)
    pl = torch.sigmoid(-eta - h)
    return torch.stack([pl, torch.clamp(1 - pw - pl, min=1e-9), pw], 1)


def fit(scale, phase, mask):
    X = T(xt(scale))
    names = ["a0", "a1", "g", "w0", "w1"] + (["a2", "w2"] if phase else [])
    init = {"a0": 0.5, "a1": 0.0, "g": 0.5, "w0": -1.5, "w1": 0.0, "a2": 0.0, "w2": 0.0}
    p = {k: torch.tensor(init[k], dtype=torch.float64, requires_grad=True) for k in names}
    idx = torch.tensor(np.where(mask)[0])
    opt = torch.optim.LBFGS(list(p.values()), max_iter=500, line_search_fn="strong_wolfe")

    def closure():
        opt.zero_grad()
        pr = probs(p, X, idx, phase)
        loss = -torch.log(pr[torch.arange(len(idx)), Y[idx]]).mean()
        loss.backward()
        return loss

    opt.step(closure)
    out = {k: float(v) for k, v in p.items()}
    for k in ("a2", "w2"):
        out.setdefault(k, 0.0)
    with torch.no_grad():
        pr = probs({k: torch.tensor(v, dtype=torch.float64) for k, v in out.items()}, X, torch.arange(len(y)), True).numpy()
    return out, pr


def evaluate(pr, mask):
    pr, yy = pr[mask], y[mask]
    ll = -np.mean(np.log(np.clip(pr[np.arange(len(yy)), yy], 1e-12, 1)))
    return ll, np.mean((pr[:, 2] + 0.5 * pr[:, 1] - ((yy == 2) + 0.5 * (yy == 1))) ** 2)


# ---------------------------------------------------------------- choose the variant on validation games
print(f"\n{'scale':>6s} {'material':>9s} {'val log loss':>13s}")
best = None
for sc in (0.0, 2.0, 4.0, 8.0):
    for phase in (False, True):
        ll = evaluate(fit(sc, phase, fit_set)[1], val)[0]
        print(f"{sc:6.1f} {str(phase):>9s} {ll:13.4f}")
        if best is None or ll < best[0] - 1e-4:
            best = (ll, sc, phase)
_, scale, phase = best
params, pr = fit(scale, phase, train)
params["scale"] = scale
chosen = (f"saturation scale {scale:g}" if scale > 0 else "linear in pawns") + (" + material" if phase else "")
print("chosen:", chosen, params)

sf_pr = np.stack([L, D, W], 1)
report = ["| Model (held-out test games) | Log loss | Brier (expected score) |", "| --- | --- | --- |"]
for name, P in [("Stockfish W/D/L as is", sf_pr), (f"Calibrated ({chosen})", pr)]:
    ll, br = evaluate(P, test)
    report.append(f"| {name} | {ll:.4f} | {br:.4f} |")
report += ["", "| Band (test games) | Positions | Actual score | Predicted | Draw rate | Predicted draws | Log loss: calibrated | Log loss: Stockfish |",
           "| --- | --- | --- | --- | --- | --- | --- | --- |"]
for lo in range(800, 2400, 200):
    b = test & (mean_r >= lo) & (mean_r < lo + 200)
    if b.sum() == 0:
        continue
    yy = y[b]
    report.append(f"| {lo}–{lo + 199} | {b.sum()} | {np.mean((yy == 2) + 0.5 * (yy == 1)):.3f} | {np.mean(pr[b, 2] + 0.5 * pr[b, 1]):.3f} | "
                  f"{np.mean(yy == 1):.3f} | {np.mean(pr[b, 1]):.3f} | {evaluate(pr, b)[0]:.4f} | {evaluate(sf_pr, b)[0]:.4f} |")

# ---------------------------------------------------------------- plot: calibration by rating group
fig, axes = plt.subplots(1, 2, figsize=(10.5, 4.4), sharey=True)
for ax, (title, P) in zip(axes, [("Stockfish's W/D/L as is", sf_pr), (f"Calibrated ({chosen})", pr)]):
    for label, lo, hi, color in GROUPS:
        b = test & (mean_r >= lo) & (mean_r < hi)
        sp = P[b, 2] + 0.5 * P[b, 1]
        st = (y[b] == 2) + 0.5 * (y[b] == 1)
        pts = np.array([(sp[i].mean(), st[i].mean(), st[i].std(ddof=1) / np.sqrt(len(i))) for i in np.array_split(np.argsort(sp), 10)])
        ax.errorbar(pts[:, 0], pts[:, 1], yerr=1.96 * pts[:, 2], color=color, lw=2, marker="o", ms=5, capsize=0,
                    elinewidth=1.1, label=f"ratings {label}", markeredgecolor=SURFACE, markeredgewidth=1.3)
    ax.plot([0, 1], [0, 1], ls="--", lw=1, color=INK2, label="perfect calibration")
    ax.set_xlim(0, 1)
    ax.set_ylim(0, 1)
    ax.set_title(title, loc="left", fontsize=11, color=INK)
    ax.set_xlabel("Predicted expected score, side to move (decile mean)")
axes[0].set_ylabel("Actual score in the game (95% CI)")
axes[0].legend(frameon=False, loc="upper left")
fig.suptitle("Leaf values on held-out Lichess games: Stockfish vs. rating-calibrated", x=0.06, ha="left", color=INK)
fig.tight_layout()
fig.savefig(PLOTS / "leaf_calibration.png", dpi=150)

# ---------------------------------------------------------------- what it means
print("\nStockfish eval (pawns) -> human expected score (equal ratings, material 40)")
for R_ in (800, 1100, 1500, 1900, 2300):
    zz = (R_ - 1500) / 400
    cells = []
    for e in (-3, -1, 0.5, 1, 2, 3, 5, 9):
        xe = scale * np.tanh(e / scale) if scale > 0 else e
        eta = (params["a0"] + params["a1"] * zz) * xe
        h = np.exp(params["w0"] + params["w1"] * zz)
        pw, pl = 1 / (1 + np.exp(-(eta - h))), 1 / (1 + np.exp(-(-eta - h)))
        cells.append(f"{e:+g} -> {pw + 0.5 * (1 - pw - pl):.2f}")
    line = f"  {R_}: " + "  ".join(cells)
    print(line)
    report.append("" if R_ == 800 else "")
    report.append(f"Rating {R_}, eval -> expected score: " + ", ".join(cells))

out = {
    "name": "lichess-2026-10-ordered-logit-v1",
    "variant": chosen,
    "params": params,
    "definitions": {
        "x": "Stockfish eval in pawns at 50k nodes, side to move, clipped to +-10, mate +-12; x~ = scale*tanh(x/scale) if scale > 0",
        "z": "(mean rating - 1500) / 400",
        "dr": "(rating to move - other rating) / 400",
        "m": "(material - 40) / 40, P=1 N=B=3 R=5 Q=9, both sides",
        "eta": "(a0 + a1*z + a2*m) * x~ + g*dr",
        "h": "exp(w0 + w1*z + w2*m)",
        "probabilities": "win = sigmoid(eta - h), loss = sigmoid(-eta - h), draw = 1 - win - loss",
    },
    "data": {"positions": int(len(y)), "train": int(train.sum()), "test": int(test.sum()),
             "source": "Lichess standard rated 2026-10 (start of month), blitz/rapid/classical, both players in one 200-point band, 800-2399, plies 12/24/36/48"},
}
(R / "leaf_model.json").write_text(json.dumps(out, indent=1))
(R / "leaf_model.md").write_text("\n".join(report) + "\n", encoding="utf-8")
print("\n" + "\n".join(report).encode("ascii", "replace").decode())
