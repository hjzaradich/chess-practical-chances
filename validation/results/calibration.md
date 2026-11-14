## T calibration

Realized loss = X's own expected-score loss over its next k moves (Stockfish v* before minus after each move).

| Band | Setting | Pairs | Mean T | Mean realized | Calibration error | Relative error | Slope (realized ~ T) | Spearman |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1400–1600 | k1_p0.8 | 792 | 0.054 | 0.069 | 0.025 | 0.37 | 0.76 | 0.53 |
| 1400–1600 | k1_p0.9 | 792 | 0.058 | 0.069 | 0.022 | 0.32 | 0.75 | 0.50 |
| 1400–1600 | k1_p0.95 | 792 | 0.061 | 0.069 | 0.019 | 0.28 | 0.74 | 0.49 |
| 1400–1600 | k2_p0.8 | 792 | 0.094 | 0.127 | 0.045 | 0.36 | 0.81 | 0.57 |
| 1400–1600 | k2_p0.9 | 792 | 0.102 | 0.127 | 0.044 | 0.34 | 0.80 | 0.56 |
| 1400–1600 | k2_p0.95 | 792 | 0.108 | 0.127 | 0.044 | 0.34 | 0.79 | 0.56 |
| 1900–2100 | k1_p0.8 | 794 | 0.054 | 0.063 | 0.018 | 0.29 | 0.80 | 0.50 |
| 1900–2100 | k1_p0.9 | 794 | 0.059 | 0.063 | 0.016 | 0.26 | 0.81 | 0.49 |
| 1900–2100 | k1_p0.95 | 794 | 0.061 | 0.063 | 0.016 | 0.26 | 0.81 | 0.49 |
| 1900–2100 | k2_p0.8 | 794 | 0.102 | 0.126 | 0.044 | 0.35 | 0.68 | 0.51 |
| 1900–2100 | k2_p0.9 | 794 | 0.109 | 0.126 | 0.039 | 0.31 | 0.68 | 0.51 |
| 1900–2100 | k2_p0.95 | 794 | 0.114 | 0.126 | 0.037 | 0.29 | 0.68 | 0.50 |

| Setting | Relative error, 90% bootstrap interval (mean over bands) |
| --- | --- |
| k1_p0.8 | 0.27 – 0.43 |
| k1_p0.9 | 0.25 – 0.40 |
| k1_p0.95 | 0.22 – 0.39 |
| k2_p0.8 | 0.31 – 0.43 |
| k2_p0.9 | 0.27 – 0.40 |
| k2_p0.95 | 0.26 – 0.39 |

Relative error = calibration error / mean realized loss, so k = 1 (one move) and k = 2 (two moves) compare fairly.
Best (lowest mean relative error across bands): **k1_p0.95**

## Diagnostics tuning (ε, δ)

Positions where X is to move; target = X's realized loss on that one move. Spearman with trap mass should be positive, with stable_moves negative.

| ε | Spearman(stable_moves, loss) |
| --- | --- |
| 0.0025 | -0.456 |
| 0.005 | -0.463 |
| 0.01 | -0.432 |
| 0.02 | -0.401 |
| 0.03 | -0.350 |
| 0.05 | -0.327 |
| 0.08 | -0.266 |

| δ | Spearman(trap_mass, loss) |
| --- | --- |
| 0.01 | +0.547 |
| 0.02 | +0.538 |
| 0.03 | +0.514 |
| 0.05 | +0.482 |
| 0.1 | +0.424 |
| 0.15 | +0.411 |
| 0.2 | +0.377 |
| 0.3 | +0.296 |

Best: ε = **0.005**, δ = **0.01**

## D calibration (ply 30, after move 15)

| Band | Games | Draw rate | Brier: constant | Brier: Maia D | Brier: Stockfish D | AUC: Maia D | AUC: Stockfish D |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1400–1600 | 2791 | 0.034 | 0.0332 | 0.0329 | 0.2608 | 0.61 | 0.55 |
| 1900–2100 | 2877 | 0.055 | 0.0519 | 0.0516 | 0.3545 | 0.64 | 0.56 |
