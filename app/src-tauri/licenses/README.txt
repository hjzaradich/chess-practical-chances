Trickiness - third-party components and licenses

This application is free software, licensed under the GNU Affero General Public License,
version 3 or later (AGPL-3.0-or-later).

Bundled or used components:

- Stockfish 19 (chess engine, shipped as a separate program next to the app)
  License: GNU General Public License v3 (Stockfish-COPYING.txt). Authors: Stockfish-AUTHORS.txt.
  Exact source for the bundled binary: https://github.com/official-stockfish/Stockfish/releases/tag/sf_19

- Maia-3 (human move prediction; the 5M model, exported to ONNX with move history as an input)
  Inference code license: GNU Affero General Public License v3 (Maia3-LICENSE.txt).
  Source: https://github.com/CSSLab/maia3   Model weights: https://huggingface.co/UofTCSSLab/Maia3-5M
  Our input encoding is ported from the official package.

- chessground (board), GPL-3.0-or-later: https://github.com/lichess-org/chessground
- ONNX Runtime (MIT), Tauri (MIT/Apache-2.0), React (MIT), Recharts (MIT),
  shakmaty and pgn-reader (GPL-3.0-or-later), rusqlite/SQLite (MIT/public domain).
