# Chess Trickiness App — Build Spec for Claude Code

Nov 11, 2026 · @Henry

## How to use this doc

Claude Code: build this in phases, in order, and stop at each phase's gate to report results before starting the next. This file is the source of truth; save it in the repo as `SPEC.md`.

- Read the whole spec before writing code. Phase 0 exists because some design choices depend on what the libraries actually expose.
- Log every decision that departs from this spec, or settles an open question, in `DECISIONS.md` with a one-line reason.
- Tasks inside a phase marked **(parallel)** don't depend on each other and can run in separate sessions.
- Do not skip Phase 2. If the metric fails validation, the GUI isn't worth building yet.
- No time estimates or schedules. Report what's done, what's blocked, and what you need from Henry.

## Purpose

The app loads a PGN and shows three numbers per move alongside the engine eval: the draw rate D, and a trickiness score for each player, T\_White and T\_Black. Both are tied to a chosen rating.

An engine eval assumes perfect play from here on, so it hides two things this app is meant to show:

- **Swinginess.** A 0.00 can be a dead draw or a knife fight. D tells them apart.
- **Difficulty per side.** A +8 with one only-move is very different from a +8 where everything wins. T measures how many points a human at a given rating is expected to lose from here, compared with the eval, separately for each side.

Sanity checks the finished metric must pass:

| Position type | T\_White | T\_Black |
| --- | --- | --- |
| Rook vs. rook, or blocked pawns where neither king can get in | close to 0 | close to 0 |
| Balanced but sharp opening | medium | medium |
| Rook and bishop vs. rook (theoretical draw) | close to 0 for the attacker | above the attacker's, and falling as rating rises |
| Opening trap | low for the trapper | high for the victim |

T should shrink as the rating rises.

T measures the danger in a player's next k decisions, not over a whole ending. In R+B vs R the defender rarely loses in the next two moves (at most a few percent per move), but the small risk compounds over a long defence. So the defender's T stays small at each position, and the difficulty shows as a long run of moderate values across the game (DECISIONS 2.9).

## Definitions and math

All values are expected scores in \[0, 1\] from the named player's point of view: win = 1, draw = d (default 0.5), loss = 0. Never compute in centipawns: +5 and +8 are both "won," and centipawn gaps overstate differences that don't matter.

**Inputs per position**

- **v(n)**: Stockfish's objective value of position n. Use Stockfish's WDL output (`UCI_ShowWDL`) and convert it: v = W + d·D. This is the leaf evaluator everywhere.
- **v\***: v at the root, the normal eval in expected-score form.
- **P\_R(m | n)**: Maia-3's probability that a player rated R plays move m in position n.
- **D\_R(n)**: Maia-3's value-head draw probability at rating R. This is the swinginess number: low D means swingy.

**Trickiness T\_X** for player X at rating R\_X, with horizon k (X's own decisions) and probability cutoff p:

Build a tree from the root. At X's nodes, expand Maia's moves in order of probability until they cover a share p of the probability, and renormalize their probabilities to P̃. At the opponent's nodes, play Stockfish's best move only. Score leaves with v.

```latex
V_X(n) = \begin{cases} \sum_m \tilde P_{R_X}(m \mid n)\, V_X(n \cdot m) & \text{X to move} \\ V_X(n \cdot m^{\ast}_{SF}) & \text{opponent to move} \\ v(n) & \text{leaf} \end{cases}
```

```latex
T_X = v^{\ast} - V_X(\text{root})
```

The opponent plays perfectly on purpose, so any points lost come only from X's own decisions. Report T raw, even when engine noise makes it slightly negative; don't clip it.

**Diagnostics** (explain why T is high; not part of T):

- **Stability**, N\_ε: the number of moves within ε of the best, from Stockfish MultiPV.
- **Trap mass**: the share of Maia's probability on moves that lose more than δ.
- **Hiddenness** (optional): disagreement between a shallow and a deep Stockfish ranking of the top moves.

**Starting parameter values**, all user-settable and to be tuned in Phase 2:

| Parameter | Meaning | Start |
| --- | --- | --- |
| k | X's decisions to look ahead | 2 |
| p | Maia probability mass to expand | 0.90 |
| ε | stability window, expected score | 0.03 |
| δ | loss counted as a trap, expected score | 0.10 |
| d | value of a draw | 0.5 |
| R\_White, R\_Black | ratings | from PGN headers, else 1500 |

## Architecture and stack

All chess math lives in an analysis core that runs without a GUI. The desktop app only starts the core and draws what it streams back.

&#91;embedded content: architecture · core, engines, GUI\]

The tree evaluator is the only part that combines engine outputs. Everything it needs comes from Stockfish, Maia or the cache.

- **Core:** a Rust crate built into the Tauri app if the Phase 0 Rust/ONNX spike succeeds; otherwise a Python package run as a sidecar process. Either way it also works as a standalone command-line tool.
- **Stockfish:** a bundled binary, run as child processes over UCI.
- **Maia-3:** ONNX Runtime. Maia3-5M is the default, downloaded on first run; larger models are optional.
- **Cache:** SQLite on disk.
- **Contract between core and GUI:** JSON Lines, one object per ply (schema in Phase 1).
- **GUI:** Tauri with a web front end. Default to React unless Henry says otherwise.

## Phase 0: feasibility spikes

Settle the questions the design depends on with small throwaway experiments, kept in `spikes/`. Write no app code yet.

1. **(parallel) Maia-3 in Rust.** Load the Maia3-5M ONNX export with ONNX Runtime from Rust (the `ort` crate). For about 10 test positions at 3 ratings, match the move probabilities and W/D/L from the official Python package (`CSSLab/maia3`). The hard part is copying Maia's input encoding and how the rating is fed in. If that can't be matched exactly, say so and fall back to a Python sidecar.
2. **(parallel) Stockfish over UCI.** Run Stockfish as a child process. Confirm MultiPV and `UCI_ShowWDL` output, and measure time per position at a few node limits on Henry's machine.
3. **(parallel) Maia's draw estimate.** Print D\_R for the Purpose table's sanity positions at ratings 1100, 1500 and 1900. Check that rook vs. rook comes out drawish and a sharp Sicilian less so.
4. **Cost of one game.** Using 1–3, count tree positions per ply at k = 2, p = 0.90, and measure the runtime for an 80-ply game, CPU only. Repeat at k = 1 and k = 3.

**Gate:** report back with (a) Rust/ONNX or Python sidecar, with the reason, (b) the measured runtimes per setting, and (c) recommended default k, p and node limits. Henry approves before Phase 1 starts.

## Phase 1: analysis core and CLI

Build the engine-facing core as a library with a command-line tool on top. Use the language Phase 0 chose: Rust (with `shakmaty` and `pgn-reader`) or Python (with `python-chess`). It must run with no GUI.

1. **(parallel) Engine wrappers.** A Stockfish pool, one process per worker thread, with MultiPV, WDL and node limits. A Maia wrapper that returns P\_R and W/D/L per position, batching positions to cut overhead.
2. **(parallel) Cache.** A transposition cache keyed by position (FEN without move counters) plus engine settings, stored on disk in SQLite, so re-analyzing a game or a common opening costs almost nothing.
3. **Tree evaluator.** Implements V\_X and T\_X exactly as defined above. Unit-test it with a fake evaluator on small hand-built trees whose answers are worked out by hand, including the made-up example: a move worth 0.45 objectively, where the opponent errs 40% of the time into 0.85, is worth 0.61.
4. **Per-ply output.** For each ply, write one JSON object and stream it as a line as soon as it's done, so the GUI can show progress later:

```json
{"ply": 23, "san": "Nxe5", "fen": "...", "side_to_move": "black",
 "v_star": 0.52, "draw_rate": 0.30,
 "trickiness": {"white": 0.04, "black": 0.18},
 "diagnostics": {"white": {"stable_moves": 4, "trap_mass": 0.06},
                 "black": {"stable_moves": 1, "trap_mass": 0.41}},
 "likely_moves": [{"san": "Qe7", "p": 0.38, "v": 0.50}, {"san": "d6", "p": 0.27, "v": 0.21}],
 "settings": {"k": 2, "p": 0.9, "nodes": 200000, "d": 0.5}}
```

5. **CLI.** Two commands:
   - `trickiness analyze game.pgn --white-elo 1500 --black-elo 1600 --out game.jsonl`
   - `trickiness position "<FEN>" --elo 1500`, for quick checks and Phase 2

The example values above are placeholders to show the format, not expected results.

**Gate:** the unit tests pass, and one full game runs end to end from the command line.

## Phase 2: validation

This phase checks that T and D measure something real before any UI exists. Tune the parameters here, not by eye.

1. **(parallel) Sanity positions.** Collect FENs for each row of the Purpose table in `validation/positions.yaml`; Henry can suggest specific positions. Tests check *orderings*, not exact values: a sharp opening's T is greater than rook vs. rook's, and the rook-and-bishop defender's T is greater than the attacker's. T falls as rating rises. Rook vs. rook's D is higher than a sharp opening's.
2. **(parallel) Real games.** Take rated standard games from the [Lichess open database](https://database.lichess.org/) in two rating bands (for example 1400–1600 and 1900–2100), a few thousand games each.
   - **T test:** for each ply, compare predicted T\_X with X's *realized* loss: the sum of v\* before minus v\* after each of X's next k moves, measured with Stockfish. Group positions into buckets by predicted T and plot average realized loss per bucket. A good metric gives a rising, roughly diagonal line.
   - **D test:** group positions around move 15 by D, then compare with how often those games actually ended in a draw.
3. **Tune.** Pick k, p, ε and δ that give the best fit in step 2, and record them in `DECISIONS.md`.

**Gate:** report the sanity-test results and the calibration plots. Henry decides whether the metric is meaningful. If it isn't, stop and discuss before Phase 3.

## Phase 3: desktop GUI

Build a Tauri app that runs the Phase 1 core and displays its JSON output. The GUI does no chess math of its own.

1. **(parallel) Load and settings.** Open a PGN file or paste one. Ratings are prefilled from the PGN headers and can be edited. Offer Quick, Standard and Deep presets, which map to the k, p and node limits settled in Phase 0. On first run, download the Maia model and show its size before downloading. Bundle Stockfish as a sidecar.
2. **(parallel) Board and move list.** Use an existing board component. Check licenses before choosing (chessground is GPL-3.0; react-chessboard is MIT) and record the choice in `DECISIONS.md`.
3. **Game chart.** One chart over the game's plies with four lines: v\* as expected score, D, T\_White and T\_Black. Clicking a point jumps the board to that ply.
4. **Position breakdown.** For the selected ply, for each player: Maia's likely moves with probability and value, moves that lose more than δ highlighted, the stable-move count, and trap mass. Always show the settings used.
5. **Progress.** Fill the chart as each ply streams in, with a cancel button. Results save next to the cache, so reopening a game is instant.
6. **Export** results as JSON or CSV.

Target Windows first, since that's Henry's machine, but don't use anything that blocks macOS or Linux builds.

**Gate:** Henry analyzes a few of his own games and signs off on the display.

## Phase 4: practical-move suggestions

At any position, show the objective best move next to the practical best move against this particular opponent, and say what the practical choice is betting on.

For each candidate move m of player X (Stockfish's top 5 plus Maia's top 5), compute a practical value Q(m). Model the opponent's replies with Maia at their rating, blended with Stockfish's best reply so that a wrong model can't fully take over. Model X's own later moves with Maia at X's rating, made somewhat sharper than usual (temperature τ below 1), because X can't count on finding every only-move either. Score leaves with u = W + d·D.

```latex
Q(m) = \sum_{r} \big[(1-\lambda)\,P_{R_{opp}}(r) + \lambda\,\mathbf{1}\{r = r^{\ast}_{SF}\}\big]\; V^{\tau}_X(n \cdot m \cdot r)
```

- λ (start 0.3) sets how far to trust the opponent model. At λ = 1 this is ordinary engine play.
- A draw-value slider controls d: 0.5 normally, 0 in a must-win game, where the search then prefers high-variance moves on its own.
- Display: "Objective best: Nf3 (0.52). Practical best: g4 (0.58, +0.06). Relies on: Black missing …Qh4, which Maia says 1500s miss 45% of the time." (Example wording only; the numbers are placeholders.)
- A move is shown as the practical best only when it beats the objective best by a clear margin (0.02 expected score, measured against this opponent). Otherwise the display says there's no practical edge over the engine move. On a human scale most sensible moves sit within a few hundredths of each other, so smaller differences are noise (DECISIONS 4.8).

**Gate:** on a few well-known opening traps whose trap move costs something objectively, the trap move's practical margin over the best alternative falls as the opponent's rating rises. (Restated by Henry on 2026-11-13, DECISIONS 4.8. The original wording, "the practical choice points at the trap at low ratings and stops doing so at high ratings", turned on whether a trap is exactly the top move, a knife-edge test on margins smaller than engine noise.)

## Constraints and working rules

- **CPU first.** Everything must run without a GPU. Lc0 is an optional add-on (a second opinion on D, using its `WDLCalibrationElo` option), never a requirement.
- **Licenses.** Stockfish is GPL-3.0, and bundling it affects how the app can be distributed. Check Maia-3's license and the board component's, and record what that means in `DECISIONS.md` before Phase 3.
- **Keep the math pure.** The tree evaluator and metric code take an evaluator interface and do no input/output, so they can be tested with fakes. Engines plug in behind that interface.
- **Traceable numbers.** Every number shown records the settings, engine versions and model that produced it.
- **No extra features.** Build what's in this spec. Put ideas in `IDEAS.md` instead of building them.
- **Be blunt.** If something in this spec is wrong, impractical or untestable, say so with evidence instead of working around it quietly. Henry would rather drop a bad idea early.

## References

**Engines and models**

- [Maia-3 repository](https://github.com/CSSLab/maia3): official inference code, UCI wrapper, model aliases
- [Introducing Maia-3](https://lichess.org/@/ashtonanderson/blog/introducing-maia-3-free-and-open-source/vCPPRtX3): what the model offers (human eval by rating, position danger)
- [Maia3 ONNX export](https://huggingface.co/bqrio/maia3-onnx): a community export with move-policy and W/D/L outputs. Check it against the official package in Phase 0.
- [Lc0 WDL rescale and contempt](https://lczero.org/blog/2023/07/the-lc0-v0.30.0-wdl-rescale/contempt-implementation/): for the optional Lc0 second opinion
- [Lichess open database](https://database.lichess.org/): games for Phase 2

**Prior work on the idea**

- [Expected Scores for Chess Positions](https://chessenginelab.substack.com/p/expected-scores-for-chess-positions): Maia and Lc0 contempt compared with real Lichess results
- [Evaluating Sharpness using LC0's WDL](https://chessenginelab.substack.com/p/evaluating-sharpness-using-lc0s-wdl)
- [Regan and Haworth, Intrinsic Chess Ratings](https://core.ac.uk/download/356043.pdf): probabilistic models of human move choice
- [Carmel and Markovitch, Incorporating Opponent Models into Adversary Search](https://mlanthology.org/aaai/1996/carmel1996aaai-incorporating): the basis for Phase 4
- [Gordon and Reda, Trappy Minimax](https://scholars.csus.edu/esploro/outputs/conferenceProceeding/Trappy-Minimax---using-Iterative-Deepening/99257847434701671)

**GUIs to borrow from (read, don't fork)**

- [En Croissant](https://github.com/franciscoBSalgueiro/en-croissant): Tauri, Rust and React chess GUI
- [Nibbler](https://github.com/rooklift/nibbler): how to parse and display Lc0's W/D/L output
