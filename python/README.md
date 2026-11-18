# practical-chances

How likely is a human, not an engine, to go wrong from this position? This is the Python API of
[Trickiness](https://github.com/hjzaradich/chess-practical-chances). It combines Stockfish
(objective play) with Maia-3 (how people at a given rating actually move).

For every position it gives:

- **v\***: Stockfish's expected score for White.
- **Draw rate**: how often Maia expects games like this to be drawn.
- **Trickiness T** for each player: the expected points they lose over their next few decisions,
  compared with perfect play.
- **The practical move**: the move that scores best against *this* opponent, the reply they have
  to miss for it to work, and how often players at their rating miss it.

The analysis runs in Rust (through PyO3) with the GIL released. Results come back as typed,
frozen dataclasses and convert to pandas DataFrames.

## Install

From a checkout (needs a Rust toolchain):

```bash
cd python
pip install maturin
maturin develop --release --extras pandas
```

You also need [Stockfish](https://stockfishchess.org/download/) on your PATH, or set
`TRICKINESS_STOCKFISH`. The Maia model is downloaded once (21 MB, checksum-verified):

```python
import practical_chances as pc

pc.download_model()
```

## Use

```python
from pathlib import Path
import practical_chances as pc

with pc.Engine(preset="quick") as eng:
    # The practical move against an 800-rated opponent
    r = eng.practical_move("e4 e5 Bc4 Nc6", white_elo=800, black_elo=800)
    print(r.summary())
    # Objective best: Nf3 (0.52).
    # Practical best: Qh5 (0.56, +0.06 over Nf3 against this opponent).
    # Relies on: Black missing g6, which 800s miss 19% of the time. If they find it: 0.47.
    print(r.to_frame())

    # A whole game, one row per ply
    game = eng.analyze_game(Path("game.pgn"), progress=lambda ply: print(ply.ply, ply.san))
    df = game.to_frame()
    print(df[["move", "v_star", "draw_rate", "t_white", "t_black"]])
    for r in game.trickiest("black", n=3):
        print(r.ply, r.san, r.trickiness.black)
```

- **Presets:** `quick`, `standard` and `deep` set the look-ahead and Stockfish's node count,
  as in the app.
- **Cache:** evaluations go into a per-user SQLite file. Pass `cache=False` to keep them in
  memory only, or pass a path.
- **Traceability:** every result carries `settings`, which records the engine and model
  versions it came from.

## Develop

```bash
pip install -e ".[dev]"   # or: maturin develop --release --extras dev
pytest                    # engine tests are skipped if Stockfish or the model is missing
ruff check . && mypy
```

The package layout:

- `src/practical_chances/`: the typed API, including `models.py` (dataclasses),
  `engine.py` and `files.py` (finding and downloading the engines).
- `../crates/py/`: the PyO3 bindings, a thin layer over `../crates/core/`.

License: AGPL-3.0-or-later. Maia-3 is AGPL and Stockfish is GPL; see the main repository.
