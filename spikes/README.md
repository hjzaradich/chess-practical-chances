# Phase 0 spikes — report

Throwaway experiments that settle the design questions in `SPEC.md` Phase 0. None of this is app code.
Measured on Henry's laptop: Intel i7-8565U (4 cores / 8 threads, 1.8 GHz), 16 GB RAM, Windows 11, CPU only.

## Gate answers

**(a) Core language: Rust.** The Rust/ONNX spike reproduces the official Python package exactly enough to treat the two as the same model. No sidecar is needed.

**(b) Runtimes:** see the tables below. At the recommended Standard setting (k = 2, p = 0.90, 50k nodes), an 80-ply game takes **about 3 to 3.5 minutes** on this laptop. A game is analysed once; results stream in ply by ply and are cached, so reopening it is instant.

**(c) Recommended defaults** (the presets the GUI offers in Phase 3):

| Preset | k | p | Stockfish nodes per search | 80-ply game on this laptop |
| --- | --- | --- | --- | --- |
| Quick | 1 | 0.90 | 50k | 0.7–0.9 min |
| Standard (default) | 2 | 0.90 | 50k | 3.0–3.4 min |
| Deep | 2 | 0.90 | 200k | 12.4–12.8 min |

k = 3 is not offered: at 50k nodes it needs about 26 minutes per game here, and its trees reach 2,200 positions in a single ply.
Deep raises the node count rather than p, because Stockfish noise, not the probability cutoff, is the bigger error at 50k (see game B).
Phase 2 tunes these values; they're starting points. The main open question for Phase 2 is whether 50k nodes is accurate enough for Standard.

## Spike 1 — Maia-3 in Rust (`s1_export_and_reference.py`, `s1_maia_rust/`)

- Official package: `CSSLab/maia3`, model Maia3-5M (`UofTCSSLab/Maia3-5M`).
- **What the model actually takes as input** (from the official code, not the spec):
  - The **last 8 positions**, oldest first, each as 64 squares × 12 piece channels. Each slot is mirrored by its own side to move, and short histories are padded at the front with the earliest position. There's no castling, en-passant or side-to-move channel.
  - **Two ratings**: the mover's and the opponent's. The rating enters as a linear blend of two learned vectors.
  - Outputs: 4,352 move logits (with promotions), and W/D/L as [loss, draw, win] for the side to move.
- I exported the official weights to ONNX myself, with history as an input. The community 5M export can't take history (see below).

| Check (14 positions × ratings 1100/1500/1900 = 42 cases) | Max abs difference |
| --- | --- |
| Our numpy encoder vs the official tokenizer | 0 |
| Our ONNX vs official PyTorch, move probabilities | 1.8e-6 |
| Our ONNX vs official PyTorch, W/D/L | 3.9e-7 |
| **Rust (shakmaty encoding + `ort`) vs official PyTorch, move probabilities** | **1.8e-6** (top move agrees 42/42) |
| Rust vs official PyTorch, W/D/L | 3.9e-7 |
| Community `bqrio/maia3-onnx` 5M vs PyTorch with the board repeated 8× | 2.2e-6 |

The cases cover white and black to move, castling, en passant, promotions and an underpromotion capture.

- **History matters:** real history versus the board alone changes move probabilities by up to **0.18**. D changes by at most 0.007. The community 5M export repeats the current board internally, so it would give history-free probabilities. That's why we ship our own export.
- Speed (Rust, 1 thread): 20 ms per position alone, **13–14 ms per position in batches of 8–32**.

## Spike 2 — Stockfish over UCI (`s2_stockfish.py`, log in `s2_stockfish.log`)

Stockfish 19, official Windows x86-64 universal build. `UCI_ShowWDL` and `MultiPV` both work: each `info` line carries `wdl W D L` in per-mille for the side to move.

| Nodes | ms per search (1 thread, MultiPV 1) | ms (MultiPV 5) | Depth reached |
| --- | --- | --- | --- |
| 10k | 19 | 21 | 10 |
| 50k | 89 | 121 | 14 |
| 200k | 404 | 416 | 17 |
| 1M | 2,229 | 1,891 | 21 |

- Speed is about 0.5 M nodes/s per thread on this CPU, which is slow. Parallel throughput at 200k nodes: 3.0 searches/s with 1 process, 4.4 with 2, **6.1 with 4**, 6.8 with 8. Use one process per physical core.
- **Parsing pitfall:** a node-limited search often stops partway through a depth. The last line can then be a `lowerbound`/`upperbound`, and higher MultiPV slots can be left over from the previous depth. The core must handle this (see `DECISIONS.md` 0.6).
- **Eval noise:** v in sharp positions moves by 0.03–0.07 between node limits (Najdorf: 0.528 at 10k → 0.594 at 1M). Quiet and drawn positions don't move. T inherits this noise.
- Stockfish's WDL is calibrated for engine play, so its scale is compressed. In the Najdorf, +0.44 pawns comes out as W 9%, D 91%, giving v = 0.55.

## Spike 3 — Maia's draw estimate (`s3_draw.py`, log in `s3_draw.log`)

D = Maia's value-head draw probability, side to move, with the same rating for both sides.

| Position | D at 1100 | D at 1500 | D at 1900 | Stockfish W/D/L |
| --- | --- | --- | --- | --- |
| Rook vs rook | 0.68 | 0.59 | 0.56 | 4/992/4 |
| Blocked pawns, kings can't get in | 0.87 | 0.94 | 0.96 | 4/992/4 |
| R+B vs R, attacker to move | 0.47 | 0.32 | 0.26 | 16/983/1 |
| R+B vs R, defender to move | 0.56 | 0.46 | 0.45 | 2/992/6 |
| Berlin endgame (after 8…Kxd8) | 0.06 | 0.05 | 0.07 | 11/986/3 |
| Najdorf, English Attack (12…b4) | 0.017 | 0.017 | 0.021 | 218/781/1 |

- **Passes the spec check:** rook vs rook and the blocked position come out drawish, and the sharp Sicilian much less so.
- In R+B vs R, Stockfish says dead draw while Maia gives the attacker a 64% win at 1500. That's the gap T\_defender is meant to show.
- **A real limit (being blunt, as the spec asks):** D is the probability that *the game ends* in a draw. With lots of material at club level that probability is low almost regardless of how sharp the position is. The Berlin endgame, a byword for dryness, scores D = 0.05, barely above the Najdorf's 0.017. So D cleanly separates simplified, drawn endings from everything else, but has little resolution between quiet and sharp middlegames. Phase 2's D test will show whether that's acceptable. One option to raise then: report D relative to a baseline for the material and rating, rather than raw.
- The traps react to rating as hoped. Blackburne-Shilling 4.Nxe5? is played 30% of the time at 1100 and 22% at 1900. In the Englund, 4.Bf4 (which allows …Qb4+ and …Qxb2) drops from 51% to 30%.

## Spike 4 — Cost of one game (`s4_cost.py`, results in `s4_results/`)

Each ply does one root search (MultiPV 5, for v\* and stability), then the T\_White and T\_Black trees exactly as defined in the spec. Setup: 4 Stockfish processes with 1 thread each; Maia batched by tree level; a transposition cache (Stockfish keyed by FEN, Maia keyed by 8-position history).

**Game A:** 80 plies of Maia-1500 self-play. Caveat: it was decided by ply 9, so v\* saturates and T = 0 for most of it. The runtimes are still valid, because tree size depends on Maia's branching, not on the eval.

| k | p | Nodes | Total | s/ply | Tree positions/ply (mean / max) | Stockfish searches (requested → run after cache) |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 0.90 | 50k | 0.9 min | 0.66 | 14 / 26 | 1,054 → 865 |
| 2 | 0.80 | 50k | 1.8 min | 1.36 | 49 / 138 | 3,274 → 2,421 |
| 2 | 0.90 | 50k | 3.4 min | 2.51 | 90 / 298 | 6,314 → 4,755 |
| 2 | 0.95 | 50k | 6.0 min | 4.42 | 159 / 474 | 11,524 → 8,751 |
| 2 | 0.90 | 200k | 12.4 min | 9.17 | 92 / 295 | 6,438 → 4,879 |
| 3 | 0.90 | 50k | 13.1 min for 40 plies (~26 min for 80, extrapolated) | 19.15 | 600 / 2,201 | 21,188 → 14,133 |

- Stockfish dominates the cost. Maia is about 14 ms per position, and fewer than 1 in 7 tree positions need it.
- The cache saves about 25% of searches within a single game. Across games it will save more, since openings repeat.
- Cost scales with nodes almost linearly, and with the cube of the branching factor at k = 3.

**Game B: Kasparov–Topalov, Wijk aan Zee 1999** (sharp and balanced for most of the game). Its purpose is the noise check that game A couldn't give.

| k | p | Nodes | Total | s/ply | Tree positions/ply (mean / max) | Stockfish searches (requested → run after cache) |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 0.90 | 50k | 0.7 min | 0.55 | 14 / 29 | 1,061 → 712 |
| 2 | 0.80 | 50k | 1.8 min | 1.34 | 46 / 169 | 2,989 → 1,822 |
| 2 | 0.90 | 50k | 3.0 min | 2.24 | 79 / 229 | 5,417 → 3,245 |
| 2 | 0.90 | 200k | 12.8 min | 9.48 | 80 / 263 | 5,508 → 3,471 |

On a real game the cache saves about 40% of searches, because the moves actually played keep landing in the previous ply's tree.

How much T moves between settings (same game, 81 plies, values in expected score):

| Comparison | Mean \|ΔT\| (White / Black) | Max \|ΔT\| | Correlation |
| --- | --- | --- | --- |
| 50k vs 200k nodes (k = 2) | 0.066 / 0.042 | 0.72 | 0.82 / 0.80 |
| k = 1 vs k = 2 | 0.135 / 0.106 | 0.90 | 0.73 / 0.60 |
| p = 0.8 vs 0.9 | 0.101 / 0.089 | 1.39 | 0.66 / 0.54 |

**Findings from game B (blunt version):**

1. **Stockfish noise at 50k nodes is large in sharp positions.** T at 50k nodes and at 200k agrees only to correlation 0.8. Worse, **v\* itself differed by 0.06 on average (up to 0.5) between two runs with identical settings**. Each Stockfish process keeps its hash table from earlier searches, so a result depends on what that process searched before. Fix for Phase 1: clear the hash before every search. That makes every number reproducible, which "traceable numbers" requires anyway, and it keeps the cache consistent.
2. **The p = 0.8 vs 0.9 comparison is mostly that same noise.** Even v\* differs by 0.06 between those runs, so the effect of p can't be separated until searches are reproducible. Re-measure in Phase 2.
3. **k matters.** k = 1 and k = 2 correlate only 0.6–0.7. A two-move combination is invisible at k = 1. Example: through Kasparov's king hunt (plies 63–70), T\_White is 0.9–1.0 at k = 2 and 0.1–0.3 at k = 1.
4. **T is large in sharp play.** Median T at k = 2 is 0.20 and the 90th percentile 0.50, at 1500 for both sides. After 24.Rxd4!! T\_White is 0.4–0.5, and in the king hunt it reaches 1.0: every likely 1500 move leads within two moves to a position Stockfish calls lost. That matches the spec's "+8 with one only-move" case, but Phase 2 calibration has to confirm the size isn't inflated. Quiet openings sit at 0.00–0.10.
5. **v\* should come from the same kind of search as the leaves.** Here v\* came from a MultiPV-5 search, which reaches 3–4 plies less depth than a MultiPV-1 search at the same node count. That mismatch can bias T. In Phase 1, take v\* from a MultiPV-1 search and run MultiPV separately, for the stability diagnostic only.
6. Negative T shows up (minimum −0.36 at k = 1, −0.14 at k = 2) and is reported raw as the spec says. In both cases it's root-versus-leaf search noise.

## Files

- `common.py`: Maia encoding, the ONNX wrapper, and the Stockfish UCI wrapper.
- `positions.py`: test positions.
- `s1_export_and_reference.py`, `s1_python.log`, `s1_reference.json`, `s1_maia_rust/`: spike 1.
- `s2_stockfish.py`, `s2_stockfish.log`: spike 2.
- `s3_draw.py`, `s3_draw.log`: spike 3.
- `s4_cost.py`, `s4_compare.py`, `s4_results/`: spike 4.
- `_dl/` (Stockfish, model files), `_vendor/maia3` (official repo) and `.venv/` are downloads, and are not part of the project.

To reproduce, from `spikes/` after creating `.venv` (torch CPU, onnxruntime, onnx, python-chess, numpy, `pip install -e _vendor/maia3`):

```bash
.venv/Scripts/python s1_export_and_reference.py
```

```bash
cd s1_maia_rust && cargo run --release -- ../_dl/models/maia3-5m.history.fp32.onnx ../s1_reference.json
```

```bash
.venv/Scripts/python s4_cost.py --game kasparov --k 2 --p 0.9 --nodes 50000 --out s4_results/kasparov/k2.json
```
