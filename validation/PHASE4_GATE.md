# Phase 4 gate: practical move on opening traps

Nov 13, 2026. Gate (SPEC.md): *on a few well-known opening traps, the practical choice points at the trap at low ratings and stops doing so at high ratings.*

Setup: 8 trap positions (`validation/traps.yaml`) × ratings 800 / 1100 / 1500 / 1900 / 2300, both players at the same rating, Standard settings (k-equivalent horizon 1, p = 0.9, 50k nodes), λ = 0.3, τ = 0.7, d = 0.5. Script: `validation/practical_gate.py`; raw JSON in `results/practical/` and `results/practical_maia_leaves/`.

## Result: the gate fails as specified

"**trap**" = the trap move is the practical best; "#n" = its rank by practical value (the practical best in brackets).

**Spec as written: leaves scored with Stockfish's W/D/L. 1 of 8 pass.**

| Trap (move) | 800 | 1100 | 1500 | 1900 | 2300 |
| --- | --- | --- | --- | --- | --- |
| Blackburne-Shilling (…Nd4) | #7 (Bc5) | #7 (Bc5) | #7 (Nf6) | not a candidate | not a candidate |
| Englund (…Qe7) | #5 (d5) | #5 (d5) | #5 (d5) | #4 (d5) | #4 (d5) |
| Fishing pole (…Ng4) | not a candidate | not a candidate | not a candidate | not a candidate | not a candidate |
| Stafford (…Nc6) | #6 (d6) | #7 (d6) | #7 (d6) | #6 (d6) | #6 (d6) |
| Elephant (…Nbd7) | #3 (Be7) | #5 (Be7) | #3 (Be7) | #2 (Be7) | #2 (Be7) |
| Scholar's mate (Qh5) | **trap** | #2 (Nf3) | not a candidate | not a candidate | not a candidate |
| Siberian (…Ng4) | #2 (d6) | #2 (d6) | #2 (d6) | #2 (d6) | **trap** |
| Kieninger (…Ngxe5) | #4 (Qc5) | #3 (Bxd2+) | **trap** | **trap** | **trap** |

**Experiment: leaves scored with Maia's value head (the outcome of a game between these ratings). 2 of 8 pass.**

| Trap (move) | 800 | 1100 | 1500 | 1900 | 2300 |
| --- | --- | --- | --- | --- | --- |
| Blackburne-Shilling (…Nd4) | #7 | #6 | #7 | not a candidate | not a candidate |
| Englund (…Qe7) | **trap** | **trap** | **trap** | **trap** | **trap** |
| Fishing pole (…Ng4) | not a candidate | not a candidate | not a candidate | not a candidate | not a candidate |
| Stafford (…Nc6) | **trap** | **trap** | #2 | #2 | #2 |
| Elephant (…Nbd7) | #5 | #6 | #6 | #4 | #2 |
| Scholar's mate (Qh5) | **trap** | **trap** | not a candidate | not a candidate | not a candidate |
| Siberian (…Ng4) | #8 | **trap** | **trap** | **trap** | **trap** |
| Kieninger (…Ngxe5) | #6 | #6 | #2 | **trap** | **trap** |

## Why

1. **Stockfish leaves are too harsh for club players.** Stockfish's W/D/L is calibrated for engines, which convert a one-pawn edge. A trap move that costs half a pawn to a pawn therefore scores as nearly lost: Stafford 3…Nc6 at 0.02–0.11 and Shilling 3…Nd4 at 0.07, objectively. Against an 1100 that isn't true. The opponent's mistakes after the horizon are never counted, because leaves assume perfect play from there on.
2. **Maia leaves are too lenient for strong players.** Maia's outcome estimate barely punishes objectively bad moves, so at 2300 it still rates Englund 3…Qe7 at 0.53 and makes it the practical best. A 2300 is not that easy to trick.
3. **The opponent errs only on the first reply.** Most traps need two mistakes (Shilling: 4.Nxe5 and then 5.Nxf7??). Q, as specified, assumes the opponent is perfect after the first reply.
4. **Candidates can miss the trap.** Fishing-pole 4…Ng4 is in neither Stockfish's nor Maia's top 5, so it is never evaluated.
5. **Two "traps" in the gate set are objectively the best moves.** Kieninger's 8…Ngxe5 is the main line, and the Elephant's 4…Nbd7 is fine. "Stops doing so at high ratings" is the wrong expectation for them, so they should be replaced with traps that cost something objectively.

## Options (Henry decides)

- **A. Rating-calibrated leaves (recommended).** Convert Stockfish's evaluation into the expected score *for these ratings*, fitted on real Lichess games. We already have 5,668 games with a Stockfish eval at move 15 and the result (Phase 2); more can be streamed. This sits between Stockfish-harsh and Maia-lenient. It can be checked with a calibration plot like Phase 2's, and keeps the spec's structure (only the meaning of u changes). Cost: one fitting step, no extra runtime.
- **B. The opponent errs at every reply inside the horizon**, not just the first. This captures two-mistake traps, but it's about 5× slower (~1 minute per position at Standard) and goes beyond the spec's formula.
- **C. Keep the spec as written** and label the practical move "conservative": it only recommends a trap when it costs almost nothing objectively (Scholar's mate at 800).

With any option: widen the candidate list a little (e.g. Maia's top 8), and fix the gate's trap set (replace Kieninger and Elephant with traps that cost something objectively). Tuning λ, τ or the leaf model on these 8 positions alone would be overfitting. Whatever is chosen should be checked on fresh positions.

---

# Round 2 (Nov 13): option A, rating-calibrated leaves

Henry chose option A. Q's leaves are now scored by a model that maps Stockfish's evaluation to the outcome of a *human* game at the two ratings (`crates/core/src/leaf.rs`, fitted by `validation/fit_leaf_model.py`).

## The leaf model

- **Data:** 8,000 Lichess games (October 2026, blitz/rapid/classical), 1,000 per 200-point band from 800 to 2399, both players in the band. Stockfish at 50k nodes (the core's settings) at plies 12, 24, 36 and 48: 28,583 positions.
- **First attempt, and why it changed:** using Stockfish's W/D/L as the input failed. 58% of positions read below 0.02 or above 0.98, because at 50k nodes a two-pawn edge already reads as decided, and a +2 can't be told from a +9. The model now uses the evaluation in **centipawns** (mate = ±12 pawns). The core keeps cp/mate on every engine line, and the engine cache key moved to `sf2`.
- **Model:** ordered logit, symmetric between the sides, linear in pawns, with terms for the rating level, the rating gap and material on the board (both chosen on a validation slice, not on the test games).
- **Held-out test (8,482 positions):** log loss **0.80** against 7.5 for Stockfish's W/D/L, and Brier **0.211** against 0.268. Calibrated in all three rating groups (`results/plots/leaf_calibration.png`). One band is off: 1400–1599 is predicted 0.516 against an actual 0.449.
- **What it says** (equal ratings, middlegame material): +1 pawn is worth 0.54 at 800, 0.55 at 1500 and 0.57 at 2300; +3 is worth 0.62 / 0.66 / 0.70; a queen up, 0.81 / 0.88 / 0.93.

## Gate, revised trap set (v2)

Kieninger and Elephant are out (their "trap" moves are objectively best or equal). Wayward Queen 2.Qh5, Albin 2…e5 and Smith-Morra 3.c3 are in, and Maia now contributes 8 candidates instead of 5. Nothing was tuned on these positions.

| | Spec (Stockfish leaves) | Calibrated leaves |
| --- | --- | --- |
| Strict gate: trap is the practical best at ≤ 1100 and not at ≥ 1900 | 1 of 9 (Scholar's) | 1 of 9 (Wayward Queen) |
| Trap's margin over the best alternative falls from 800 to 2300 | 3 of 8 | **7 of 8** |
| …and falls (within 0.005) at every rating step | 1 of 8 | **7 of 8** |
| Mean margin of the trap move | −0.12 | −0.01 |

Trap margins with calibrated leaves (practical value of the trap move minus the best other candidate):

| Trap | 800 | 1100 | 1500 | 1900 | 2300 |
| --- | --- | --- | --- | --- | --- |
| Scholar's mate 3.Qh5 | **+0.043** | **+0.035** | **+0.029** | **+0.021** | **+0.009** |
| Wayward Queen 2.Qh5 | **+0.002** | −0.009 | −0.017 | −0.022 | −0.031 |
| Siberian …Ng4 | −0.019 | **+0.014** | **+0.004** | **+0.008** | **+0.048** |
| Blackburne-Shilling …Nd4 | −0.015 | −0.020 | −0.029 | −0.036 | −0.046 |
| Englund …Qe7 | −0.004 | −0.007 | −0.011 | −0.011 | −0.014 |
| Stafford …Nc6 | −0.033 | −0.054 | −0.065 | −0.077 | −0.087 |
| Albin …e5 | −0.007 | −0.010 | −0.013 | −0.016 | −0.022 |
| Smith-Morra 3.c3 | −0.005 | −0.002 | −0.002 | −0.002 | −0.009 |
| Fishing pole …Ng4 | not a candidate at any rating (outside Maia's top 8 and Stockfish's top 5) |

## Reading

- **Calibrated leaves fix the scale and give the right trend.** Against weaker opponents the trap moves come within a few hundredths of the best move, and their margin shrinks steadily as the rating rises, in 7 of 8 traps. With Stockfish leaves, traps lose by 0.04–0.37 and the trend is right in only 3.
- **The strict gate is a knife-edge test.** On a human scale, sensible opening moves sit within a few hundredths of each other, so whether a trap is exactly #1 depends on margins like −0.002 (Smith-Morra) or +0.002 (Wayward Queen). That's within Stockfish's noise at 50k nodes.
- **Multi-step traps can't be #1 under Q as specified.** In the Shilling, Stafford and Englund the payoff needs a *second* mistake, and Q lets the opponent err only on the first reply (option B).
- **Two traps stay preferred at 2300:** Scholar's (+0.009, essentially a tie) and Siberian (+0.048, the one trap whose trend goes the wrong way).

## Proposed next step (Henry decides)

1. **Restate the gate as the trend it is meant to measure:** the trap move's practical margin falls as the opponent's rating rises (calibrated: 7 of 8 pass). In the app, call a move the "practical best" only when it beats the objective best by a clear margin (say ≥ 0.02); otherwise show "no practical edge over the engine move". Recommended.
2. **Add option B on top:** let the opponent err on their second reply too, so multi-step traps can win. About 5× slower; it would need its own check.
3. **Keep the strict gate:** Phase 4 doesn't pass it with any of the three leaf versions tried.

---

# Round 3 (Nov 13): gate restated, and passed

Henry restated the gate as the trend it is meant to measure (SPEC.md Phase 4, DECISIONS 4.8): on traps whose trap move costs something objectively, **the trap move's practical margin over the best alternative falls as the opponent's rating rises**. Pass = the margin is lower at 2300 than at 800, and never rises by more than 0.005 (engine noise) from one rating step to the next. Scored by `validation/practical_gate.py` from the same results as round 2 (no new engine runs).

| | Spec (Stockfish leaves) | Calibrated leaves (default) |
| --- | --- | --- |
| **Restated gate (trend)** | 1 of 9 | **7 of 9** |
| Original strict test, for reference | 1 of 9 | 1 of 9 |

Fails with calibrated leaves: **fishing pole** (4…Ng4 is never a candidate: it's outside Maia's top 8 and Stockfish's top 5 at every rating) and **Siberian** (…Ng4's margin rises with rating, +0.048 at 2300). Per-trap tables: `results/practical_gate_calibrated.md`, `results/practical_gate_engine_v2.md`.

Display rule (same decision): a move is called the practical best only if it beats the objective best by at least 0.02 against this opponent; otherwise the app and CLI say there is no clear practical edge over the engine move.

Checked in the running app (debug build): at 800 vs 800 after 1.e4 e5 2.Bc4 Nc6, "Objective best: Nf3 (0.52). Practical best: Qh5 (0.56, +0.06 over Nf3 against this opponent). Relies on: Black missing g6, which Maia says 800s miss 19% of the time. If they find it: 0.47." At 2300: "no clear edge over Nf3. Qh5 gains only 0.01". The draw slider recomputes from cache. At d = 0 the objective values collapse (Stockfish's W/D/L is mostly draws), while practical values hardly move (human games are rarely drawn); both follow from the definitions.

Open, not blocking: candidate generation can miss a trap that neither engine nor Maia ranks highly (fishing pole); the Siberian's reverse trend is unexplained.
