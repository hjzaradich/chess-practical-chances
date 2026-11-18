"""The ``Engine``: a typed, Pythonic front end to the Rust core in ``_core``."""

from __future__ import annotations

import json
import os
from collections.abc import Callable, Sequence
from pathlib import Path
from types import TracebackType
from typing import Literal

from . import _core
from .files import StrPath, find_maia_model, find_stockfish, user_cache_dir
from .models import Game, GameAnalysis, PlyResult, Practical

Preset = Literal["quick", "standard", "deep"]
Leaf = Literal["calibrated", "engine", "maia"]

#: (k, p, nodes) per preset, as in the CLI and the app (DECISIONS 0.8).
PRESETS: dict[str, tuple[int, float, int]] = {
    "quick": (1, 0.9, 50_000),
    "standard": (2, 0.9, 50_000),
    "deep": (2, 0.9, 200_000),
}


def parse_pgn(pgn: str | os.PathLike[str]) -> list[Game]:
    """Every game in a PGN. Pass the text, or a ``Path`` to read it from a file.

    A game that doesn't parse keeps its place in the list, with ``error`` set.
    """
    return [Game.from_dict(g) for g in json.loads(_core.parse_pgn(_pgn_text(pgn)))]


def _pgn_text(pgn: str | os.PathLike[str]) -> str:
    # A str is PGN text; anything path-like is a file. (No guessing from the string's content.)
    if isinstance(pgn, str):
        return pgn
    return Path(pgn).read_text(encoding="utf-8", errors="replace")


def _moves_text(moves: str | Sequence[str]) -> str:
    return moves if isinstance(moves, str) else " ".join(moves)


class Engine:
    """Stockfish processes plus the Maia model, ready to analyse.

    Starting the engines takes a moment, so create one ``Engine`` and reuse it; it's a context
    manager that shuts Stockfish down on exit::

        with Engine(preset="quick") as eng:
            game = eng.analyze_game(Path("game.pgn"))
            print(game.to_frame()[["move", "v_star", "t_white", "t_black"]])

    Analyses release the GIL, so they can run from several threads on one engine.

    Args:
        stockfish: Stockfish binary (default: see :func:`find_stockfish`).
        maia_model: Maia ONNX file (default: see :func:`find_maia_model`).
        preset: ``"quick"``, ``"standard"`` or ``"deep"``: the default k and p, and the nodes.
        nodes: Stockfish nodes per search (overrides the preset's).
        workers: Stockfish processes (default: physical cores).
        hash_mb: Stockfish hash per process.
        syzygy: Syzygy tablebase folder, used if Stockfish finds tables in it.
        cache: SQLite evaluation cache. ``True`` (default): a per-user file, so repeated
            analyses are fast. ``False``: memory only. Or a path.
    """

    def __init__(
        self,
        stockfish: StrPath | None = None,
        maia_model: StrPath | None = None,
        *,
        preset: Preset = "standard",
        nodes: int | None = None,
        workers: int | None = None,
        hash_mb: int = 16,
        syzygy: StrPath | None = None,
        cache: StrPath | bool = True,
    ) -> None:
        if preset not in PRESETS:
            raise ValueError(f"preset must be one of {sorted(PRESETS)}, not {preset!r}")
        self._k, self._p, preset_nodes = PRESETS[preset]
        cache_path: Path | None
        if cache is True:
            cache_path = user_cache_dir() / "cache.sqlite"
            cache_path.parent.mkdir(parents=True, exist_ok=True)
        elif cache is False:
            cache_path = None
        else:
            cache_path = Path(cache)
        self._core = _core.Engine(
            str(find_stockfish(stockfish)),
            str(find_maia_model(maia_model)),
            nodes=nodes if nodes is not None else preset_nodes,
            workers=workers,
            hash_mb=hash_mb,
            syzygy=str(syzygy) if syzygy is not None else None,
            cache=str(cache_path) if cache_path is not None else None,
        )

    # ---------------------------------------------------------------- lifecycle

    def close(self) -> None:
        """Stop Stockfish. Waits for running analyses; safe to call twice."""
        self._core.close()

    def __enter__(self) -> Engine:
        return self

    def __exit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None:
        self.close()

    def __repr__(self) -> str:
        return repr(self._core)

    # ---------------------------------------------------------------- identity

    @property
    def stockfish_name(self) -> str:
        return self._core.stockfish_name

    @property
    def maia_id(self) -> str:
        return self._core.maia_id

    @property
    def nodes(self) -> int:
        return self._core.nodes

    def cache_stats(self) -> tuple[int, int]:
        """(hits, misses) of the evaluation cache since this engine started."""
        return self._core.cache_stats()

    # ---------------------------------------------------------------- analysis

    def analyze_game(
        self,
        pgn: str | os.PathLike[str],
        *,
        game: int = 0,
        white_elo: float | None = None,
        black_elo: float | None = None,
        k: int | None = None,
        p: float | None = None,
        d: float = 0.5,
        epsilon: float = 0.01,
        delta: float = 0.05,
        multipv: int = 5,
        progress: Callable[[PlyResult], object] | None = None,
    ) -> GameAnalysis:
        """Analyse every ply of a game.

        Args:
            pgn: PGN text, or a ``Path`` to a PGN file.
            game: Which game in the PGN (0-based).
            white_elo, black_elo: Ratings (default: the PGN headers, else 1500).
            k, p: Look-ahead and Maia probability mass (default: the engine's preset).
            d: Value of a draw (0.5; 0 for a must-win game).
            epsilon, delta: Stability window and trap threshold, in expected score.
            progress: Called with each ply's result as it finishes. Raising in it stops the
                analysis and the exception propagates; so does Ctrl+C.
        """
        callback = None
        if progress is not None:
            prog = progress

            def callback(js: str) -> None:
                prog(PlyResult.from_dict(json.loads(js)))

        lines = self._core.analyze_game(
            _pgn_text(pgn),
            game=game,
            white_elo=white_elo,
            black_elo=black_elo,
            k=k if k is not None else self._k,
            p=p if p is not None else self._p,
            d=d,
            epsilon=epsilon,
            delta=delta,
            multipv=multipv,
            on_ply=callback,
        )
        return GameAnalysis.from_json_lines(lines)

    def analyze_position(
        self,
        moves: str | Sequence[str] = "",
        *,
        fen: str | None = None,
        white_elo: float = 1500,
        black_elo: float = 1500,
        k: int | None = None,
        p: float | None = None,
        d: float = 0.5,
        epsilon: float = 0.01,
        delta: float = 0.05,
        multipv: int = 5,
    ) -> PlyResult:
        """Analyse the position after ``moves`` (SAN, from ``fen`` or the start).

        Give the moves rather than only a FEN when you have them: Maia predicts better with
        the last few moves of history.
        """
        return PlyResult.from_dict(
            json.loads(
                self._core.analyze_position(
                    fen=fen,
                    moves=_moves_text(moves),
                    white_elo=white_elo,
                    black_elo=black_elo,
                    k=k if k is not None else self._k,
                    p=p if p is not None else self._p,
                    d=d,
                    epsilon=epsilon,
                    delta=delta,
                    multipv=multipv,
                )
            )
        )

    def practical_move(
        self,
        moves: str | Sequence[str] = "",
        *,
        fen: str | None = None,
        white_elo: float = 1500,
        black_elo: float = 1500,
        lambda_: float = 0.3,
        tau: float = 0.7,
        horizon: int = 1,
        d: float = 0.5,
        p: float = 0.9,
        delta: float = 0.05,
        leaf: Leaf = "calibrated",
        margin: float = 0.02,
    ) -> Practical:
        """The objective best move and the practical best move against this opponent.

        Args:
            moves: SAN moves to the position, from ``fen`` or the start.
            white_elo, black_elo: Ratings; the opponent's decides how often they miss things.
            lambda_: Trust in Stockfish's best reply versus the opponent model (1 = engine play).
            tau: Temperature for your own later moves (below 1 = sharper than Maia).
            horizon: Your own decisions after the opponent's reply.
            d: Value of a draw.
            leaf: How end positions are scored: ``"calibrated"`` (rating-aware, default),
                ``"engine"`` or ``"maia"``.
            margin: A practical best is called clear only if it gains at least this much.
        """
        return Practical.from_dict(
            json.loads(
                self._core.practical(
                    fen=fen,
                    moves=_moves_text(moves),
                    white_elo=white_elo,
                    black_elo=black_elo,
                    lambda_=lambda_,
                    tau=tau,
                    horizon=horizon,
                    d=d,
                    p=p,
                    delta=delta,
                    leaf=leaf,
                    margin=margin,
                )
            )
        )
