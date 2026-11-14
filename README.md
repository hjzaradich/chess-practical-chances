# Chess trickiness

Loads a PGN and gives three numbers per move, alongside the engine eval: Maia's draw rate D, and a trickiness score for each player (T\_White, T\_Black) at chosen ratings. `SPEC.md` is the source of truth; departures and settled questions are in `DECISIONS.md`.

Status: Phase 3 (desktop GUI) built, awaiting Henry's sign-off on his own games. Phase 2 validation: `validation/REPORT.md`. Phase 0 findings: `spikes/README.md`.

## Layout

- `crates/core`: the analysis library (Rust). The metric code (`tree.rs`, `analysis.rs`) is pure and works through the `Oracle` interface. Stockfish, Maia and the SQLite cache sit behind that interface (`stockfish.rs`, `maia.rs`, `cache.rs`, `engines.rs`).
- `crates/cli`: the `trickiness` command.
- `app/`: the desktop app (Tauri + React). The core is linked in; Stockfish ships beside it as a sidecar.
- `crates/validate`, `validation/`: Phase 2 validation runs and reports.
- `tools/export_maia_onnx.py`: builds the Maia ONNX file the core needs, from the official weights.
- `spikes/`: Phase 0 throwaway experiments.

## Setup

1. Stockfish: put a binary at `engines/stockfish.exe` (or pass `--stockfish`, or set `TRICKINESS_STOCKFISH`). Phase 0 used Stockfish 19 (official release).
2. Maia: the core needs our ONNX export with history as an input. The community exports don't take history, so they won't work. Build it once from the official checkpoint (`UofTCSSLab/Maia3-5M`, `maia3-5m.pt`):

```bash
spikes/.venv/Scripts/python tools/export_maia_onnx.py --checkpoint maia3-5m.pt --model 5m --out models/maia3-5m.history.fp32.onnx
```

   The Python environment needs torch (CPU), onnx, onnxruntime, numpy and the official `maia3` package.
3. Optional: Syzygy tablebases. Put the files in `tablebases/` (or pass `--syzygy <dir>`, or set `TRICKINESS_SYZYGY`). Stockfish then scores endgames with that many pieces exactly. The 3–5-piece set is about 1 GB, from tablebase.lichess.ovh or the syzygy-tables.info mirrors. Results record whether tablebases were used.
4. Build:

```bash
cargo build --release
```

## Use

```bash
target/release/trickiness analyze examples/kasparov_topalov_1999.pgn --white-elo 1500 --black-elo 1600 --out game.jsonl
```

```bash
target/release/trickiness position "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 3 3" --elo 1500
```

Options shared by both commands: `--preset quick|standard|deep`, or `--k`, `--p` and `--nodes` directly. Also `--d`, `--epsilon`, `--delta`, `--multipv`, `--workers`, `--syzygy <dir>`, `--cache <file>` and `--no-cache`. Ratings come from the PGN's WhiteElo/BlackElo headers unless given, else 1500.

`analyze` writes one JSON object per ply (JSON Lines) as soon as that ply is done:

```json
{"ply": 23, "san": "Nxe5", "uci": "f3e5", "fen": "...", "side_to_move": "black",
 "v_star": 0.52, "draw_rate": 0.30,
 "trickiness": {"white": 0.04, "black": 0.18},
 "diagnostics": {"white": {"stable_moves": 4, "trap_mass": 0.06, "at_ply_offset": 1},
                 "black": {"stable_moves": 1, "trap_mass": 0.41, "at_ply_offset": 0}},
 "likely_moves": [{"san": "Qe7", "uci": "d8e7", "p": 0.38, "v": 0.50}],
 "settings": {"k": 2, "p": 0.9, "nodes": 50000, "d": 0.5, "epsilon": 0.01, "delta": 0.05, "multipv": 5,
              "white_elo": 1500, "black_elo": 1600, "stockfish": "Stockfish 19", "maia": "maia3-5m.history.fp32.onnx@f2fc76df6749"}}
```

(Placeholder values.) `ply` counts half-moves played, and `san`/`uci` is the move that led to the position. `v_star` is from White's point of view, each `trickiness` and `diagnostics` entry from that player's, and `likely_moves[].v` from the side to move's.

## Desktop app

Needs Node.js, Rust and the Stockfish binary at `engines/stockfish.exe` (copied into the app as a sidecar).

```bash
cp engines/stockfish.exe app/src-tauri/binaries/stockfish-x86_64-pc-windows-msvc.exe
```

```bash
cd app && npm install && npx tauri dev
```

Build the Windows installer (`target/release/bundle/nsis/`):

```bash
cd app && npx tauri build
```

On first run the app asks for the Maia model: "Locate model file…" picks `models/maia3-5m.history.fp32.onnx` (no download location is set up yet). App data, the cache and saved results live in `%APPDATA%\app.trickiness.desktop`.

Using it: Open or paste a PGN (pick a game if the file has several), check the ratings (prefilled from WhiteElo/BlackElo), choose Quick, Standard or Deep, and press Analyse. The chart fills as plies finish; click it or use ← → to move through the game. The breakdown shows each player's T, stable moves, trap mass and Maia's likely moves, with moves that lose more than δ flagged. Export writes JSON or CSV.

The board is playable. On a game position, playing the game's next move steps forward; any other move opens a side line, analysed position by position at the toolbar's ratings and preset ("Back to game" or Esc discards it). "New board" starts from the starting position, to play through a game or try lines without a PGN.

## Tests

```bash
cargo test --release
```

The unit tests check the tree and metrics against small hand-built trees (including SPEC.md's 0.61 example). The integration tests in `crates/core/tests/engines.rs` check Maia against the official package's outputs and check that Stockfish searches are reproducible. They skip if the model or Stockfish is missing.

## Licenses

Maia-3 is AGPL-3.0 and Stockfish is GPL-3.0, so this project is AGPL-3.0-or-later (see `DECISIONS.md`).
