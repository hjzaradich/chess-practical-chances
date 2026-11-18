"""Practical chances in chess: how likely a human, not an engine, is to go wrong.

A Python API over the Rust core of the Trickiness app (Stockfish + the Maia-3 human-move
model). Per position it gives Stockfish's expected score v*, Maia's draw rate, each player's
trickiness T, and the practical best move against an opponent of a given rating.

    >>> import practical_chances as pc
    >>> pc.download_model()                      # once, 21 MB
    >>> with pc.Engine(preset="quick") as eng:
    ...     r = eng.practical_move("e4 e5 Bc4 Nc6", white_elo=800, black_elo=800)
    ...     print(r.summary())
"""

from ._core import EngineError
from ._core import __version__ as __version__
from .engine import PRESETS, Engine, parse_pgn
from .files import NotFoundError, download_model, find_maia_model, find_stockfish, user_cache_dir
from .models import (
    Candidate,
    Diagnostics,
    Game,
    GameAnalysis,
    GameMove,
    LikelyMove,
    PlyResult,
    Practical,
    PracticalParams,
    Reply,
    Settings,
    Sides,
)

__all__ = [
    "PRESETS",
    "Candidate",
    "Diagnostics",
    "Engine",
    "EngineError",
    "Game",
    "GameAnalysis",
    "GameMove",
    "LikelyMove",
    "NotFoundError",
    "PlyResult",
    "Practical",
    "PracticalParams",
    "Reply",
    "Settings",
    "Sides",
    "download_model",
    "find_maia_model",
    "find_stockfish",
    "parse_pgn",
    "user_cache_dir",
]
