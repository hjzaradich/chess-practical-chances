# Type stubs for the native module built from crates/py (PyO3). Results are JSON strings in
# the core's serde schema; practical_chances.models turns them into dataclasses.
from collections.abc import Callable

__version__: str

class EngineError(Exception):
    """Stockfish, Maia or the analysis itself failed."""

class Engine:
    def __init__(
        self,
        stockfish: str,
        maia_model: str,
        *,
        nodes: int = 50_000,
        workers: int | None = None,
        hash_mb: int = 16,
        syzygy: str | None = None,
        cache: str | None = None,
    ) -> None: ...
    @property
    def stockfish_name(self) -> str: ...
    @property
    def maia_id(self) -> str: ...
    @property
    def nodes(self) -> int: ...
    def cache_stats(self) -> tuple[int, int]: ...
    def analyze_position(
        self,
        *,
        fen: str | None = None,
        moves: str = "",
        white_elo: float,
        black_elo: float,
        k: int = 2,
        p: float = 0.9,
        d: float = 0.5,
        epsilon: float = 0.01,
        delta: float = 0.05,
        multipv: int = 5,
    ) -> str: ...
    def analyze_game(
        self,
        pgn: str,
        *,
        game: int = 0,
        white_elo: float | None = None,
        black_elo: float | None = None,
        k: int = 2,
        p: float = 0.9,
        d: float = 0.5,
        epsilon: float = 0.01,
        delta: float = 0.05,
        multipv: int = 5,
        on_ply: Callable[[str], object] | None = None,
    ) -> list[str]: ...
    def practical(
        self,
        *,
        fen: str | None = None,
        moves: str = "",
        white_elo: float,
        black_elo: float,
        lambda_: float = 0.3,
        tau: float = 0.7,
        horizon: int = 1,
        d: float = 0.5,
        p: float = 0.9,
        delta: float = 0.05,
        leaf: str = "calibrated",
        margin: float = 0.02,
    ) -> str: ...
    def close(self) -> None: ...

def parse_pgn(text: str) -> str: ...
