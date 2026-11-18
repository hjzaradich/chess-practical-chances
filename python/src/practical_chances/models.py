"""Typed results.

The Rust core returns JSON in the schema the desktop app also reads (``crates/core/src/
analysis.rs`` and ``practical.rs``). These frozen dataclasses mirror it one to one, so results
are immutable, hashable, and checked by mypy. ``from_dict`` is the only place that knows the
JSON field names.
"""

from __future__ import annotations

from collections.abc import Iterator, Mapping, Sequence
from dataclasses import dataclass, fields
from typing import TYPE_CHECKING, Any, Generic, Literal, TypeVar

if TYPE_CHECKING:
    import pandas as pd

Color = Literal["white", "black"]
T = TypeVar("T")


def _require_pandas() -> Any:
    try:
        import pandas
    except ImportError as e:  # pragma: no cover - depends on the environment
        raise ImportError("to_frame() needs pandas: pip install 'practical-chances[pandas]'") from e
    return pandas


@dataclass(frozen=True, slots=True)
class Sides(Generic[T]):
    """One value per player."""

    white: T
    black: T

    def __getitem__(self, color: Color) -> T:
        if color == "white":
            return self.white
        if color == "black":
            return self.black
        raise KeyError(color)

    def __iter__(self) -> Iterator[T]:
        yield self.white
        yield self.black


# ------------------------------------------------------------------------------- per ply


@dataclass(frozen=True, slots=True)
class Settings:
    """Everything a number depends on, so results can be traced and reproduced."""

    k: int
    p: float
    nodes: int
    d: float
    epsilon: float
    delta: float
    multipv: int
    white_elo: float
    black_elo: float
    stockfish: str
    maia: str

    @classmethod
    def from_dict(cls, d: Mapping[str, Any]) -> Settings:
        return cls(**{f.name: d[f.name] for f in fields(cls)})


@dataclass(frozen=True, slots=True)
class LikelyMove:
    """A move Maia expects, with its probability ``p`` and value ``v`` for the mover."""

    san: str
    uci: str
    p: float
    v: float
    #: Costs more than ``Settings.delta`` against the best move.
    loses: bool = False

    @classmethod
    def from_dict(cls, d: Mapping[str, Any]) -> LikelyMove:
        return cls(d["san"], d["uci"], d["p"], d["v"], d.get("loses", False))


@dataclass(frozen=True, slots=True)
class Diagnostics:
    """Why a player's trickiness is what it is: their next decision and the likely moves there."""

    stable_moves: int
    trap_mass: float
    #: 0 if the player decides now, 1 if after the opponent's best reply.
    at_ply_offset: int
    decision_fen: str
    #: The opponent's reply leading to the decision, if it isn't now.
    via: str | None
    best_value: float
    moves: tuple[LikelyMove, ...]

    @classmethod
    def from_dict(cls, d: Mapping[str, Any]) -> Diagnostics:
        return cls(
            stable_moves=d["stable_moves"],
            trap_mass=d["trap_mass"],
            at_ply_offset=d["at_ply_offset"],
            decision_fen=d["decision_fen"],
            via=d.get("via"),
            best_value=d["best_value"],
            moves=tuple(LikelyMove.from_dict(m) for m in d["moves"]),
        )


@dataclass(frozen=True, slots=True)
class PlyResult:
    """The metrics for one position of a game (SPEC.md): v*, D, T_White and T_Black."""

    ply: int
    #: The move that led here (``None`` at the start).
    san: str | None
    uci: str | None
    fen: str
    side_to_move: Color
    #: Stockfish's expected score for White.
    v_star: float
    #: Maia's draw probability.
    draw_rate: float
    #: Expected loss over each player's next decisions, vs. v*.
    trickiness: Sides[float]
    #: ``None`` for a player who doesn't move again (the game is over).
    diagnostics: Sides[Diagnostics | None]
    likely_moves: tuple[LikelyMove, ...]
    settings: Settings

    @classmethod
    def from_dict(cls, d: Mapping[str, Any]) -> PlyResult:
        diag = d["diagnostics"]
        return cls(
            ply=d["ply"],
            san=d.get("san"),
            uci=d.get("uci"),
            fen=d["fen"],
            side_to_move=d["side_to_move"],
            v_star=d["v_star"],
            draw_rate=d["draw_rate"],
            trickiness=Sides(d["trickiness"]["white"], d["trickiness"]["black"]),
            diagnostics=Sides(
                Diagnostics.from_dict(diag["white"]) if diag.get("white") else None,
                Diagnostics.from_dict(diag["black"]) if diag.get("black") else None,
            ),
            likely_moves=tuple(LikelyMove.from_dict(m) for m in d["likely_moves"]),
            settings=Settings.from_dict(d["settings"]),
        )

    def row(self) -> dict[str, Any]:
        """A flat record, one column per metric (what ``GameAnalysis.to_frame`` is built from)."""
        w, b = self.diagnostics
        return {
            "ply": self.ply,
            "move": self.san,
            "side_to_move": self.side_to_move,
            "v_star": self.v_star,
            "draw_rate": self.draw_rate,
            "t_white": self.trickiness.white,
            "t_black": self.trickiness.black,
            "trap_mass_white": w.trap_mass if w else None,
            "trap_mass_black": b.trap_mass if b else None,
            "stable_moves_white": w.stable_moves if w else None,
            "stable_moves_black": b.stable_moves if b else None,
            "top_move": self.likely_moves[0].san if self.likely_moves else None,
            "top_move_p": self.likely_moves[0].p if self.likely_moves else None,
            "fen": self.fen,
        }


# ------------------------------------------------------------------------------- games


@dataclass(frozen=True, slots=True)
class GameMove:
    san: str
    uci: str
    #: Position after the move.
    fen: str


@dataclass(frozen=True, slots=True)
class Game:
    """One game from a PGN. ``error`` is set (and ``moves`` empty) if it didn't parse."""

    index: int
    tags: Mapping[str, str]
    start_fen: str | None
    moves: tuple[GameMove, ...]
    error: str | None = None

    @classmethod
    def from_dict(cls, d: Mapping[str, Any]) -> Game:
        return cls(
            index=d["index"],
            tags=dict(d["tags"]),
            start_fen=d.get("start_fen"),
            moves=tuple(GameMove(m["san"], m["uci"], m["fen"]) for m in d["moves"]),
            error=d.get("error"),
        )

    def __hash__(self) -> int:  # tags is a dict; hash what identifies the game
        return hash((self.index, self.start_fen, self.moves, self.error))

    def elo(self, color: Color) -> int | None:
        """The WhiteElo / BlackElo header as a number, if there is one."""
        raw = self.tags.get("WhiteElo" if color == "white" else "BlackElo", "")
        return int(raw) if raw.strip().isdigit() and int(raw) > 0 else None

    @property
    def sans(self) -> list[str]:
        return [m.san for m in self.moves]

    def __str__(self) -> str:
        t = self.tags
        return f"{t.get('White', '?')} vs {t.get('Black', '?')}, {t.get('Event', '')} {t.get('Date', '')}".strip()


@dataclass(frozen=True, slots=True)
class GameAnalysis:
    """Every ply of one game. Iterable and indexable by ply."""

    plies: tuple[PlyResult, ...]

    def __len__(self) -> int:
        return len(self.plies)

    def __iter__(self) -> Iterator[PlyResult]:
        return iter(self.plies)

    def __getitem__(self, ply: int) -> PlyResult:
        return self.plies[ply]

    @property
    def settings(self) -> Settings:
        return self.plies[0].settings

    @classmethod
    def from_json_lines(cls, lines: Sequence[str] | str) -> GameAnalysis:
        """Parse JSON Lines, as written by ``trickiness analyze`` and the app's export."""
        import json

        if isinstance(lines, str):
            lines = lines.splitlines()
        return cls(tuple(PlyResult.from_dict(json.loads(s)) for s in lines if s.strip()))

    def to_frame(self) -> pd.DataFrame:
        """One row per ply, indexed by ply. Needs pandas."""
        pandas = _require_pandas()
        df: pd.DataFrame = pandas.DataFrame([r.row() for r in self.plies]).set_index("ply")
        return df

    def trickiest(self, color: Color, n: int = 3) -> list[PlyResult]:
        """The ``n`` positions where ``color`` faced the highest trickiness."""
        return sorted(self.plies, key=lambda r: r.trickiness[color], reverse=True)[:n]


# ------------------------------------------------------------------------------- practical


@dataclass(frozen=True, slots=True)
class Reply:
    """An opponent reply to a candidate and what it's worth to the player."""

    san: str
    uci: str
    #: Maia's probability for the reply.
    p: float
    #: Weight in the practical value: (1 - λ)·p, plus λ on Stockfish's best reply.
    weight: float
    objective: float
    value: float
    is_best: bool
    #: Gives the player more than δ over the best reply.
    miss: bool

    @classmethod
    def from_dict(cls, d: Mapping[str, Any]) -> Reply:
        return cls(**{f.name: d[f.name] for f in fields(cls)})


@dataclass(frozen=True, slots=True)
class Candidate:
    san: str
    uci: str
    #: Maia's probability that the player picks it.
    p: float
    #: 1-based rank in Stockfish's MultiPV, if it's there.
    engine_rank: int | None
    #: Value against perfect play.
    objective: float
    #: Value against this opponent (Q).
    practical: float
    best_reply: str | None
    #: How often the opponent misses ``best_reply``.
    miss_chance: float
    replies: tuple[Reply, ...]

    @classmethod
    def from_dict(cls, d: Mapping[str, Any]) -> Candidate:
        return cls(
            san=d["san"],
            uci=d["uci"],
            p=d["p"],
            engine_rank=d.get("engine_rank"),
            objective=d["objective"],
            practical=d["practical"],
            best_reply=d.get("best_reply"),
            miss_chance=d["miss_chance"],
            replies=tuple(Reply.from_dict(r) for r in d["replies"]),
        )


@dataclass(frozen=True, slots=True)
class PracticalParams:
    lambda_: float
    tau: float
    d: float
    p: float
    horizon: int
    delta: float
    n_engine: int
    n_human: int
    leaf: str
    margin: float

    @classmethod
    def from_dict(cls, d: Mapping[str, Any]) -> PracticalParams:
        return cls(
            lambda_=d["lambda"],
            **{f.name: d[f.name] for f in fields(cls) if f.name != "lambda_"},
        )


@dataclass(frozen=True, slots=True)
class Practical:
    """Objective best vs. practical best move against a specific opponent (Phase 4)."""

    fen: str
    side_to_move: Color
    #: Sorted by practical value, best first.
    candidates: tuple[Candidate, ...]
    objective_best: Candidate
    practical_best: Candidate
    #: Q(practical best) - Q(objective best).
    edge: float
    #: The practical best differs and its edge clears ``params.margin``.
    clear: bool
    params: PracticalParams
    white_elo: float
    black_elo: float
    leaf_model: str | None

    @classmethod
    def from_dict(cls, d: Mapping[str, Any]) -> Practical:
        cands = tuple(Candidate.from_dict(c) for c in d["candidates"])
        return cls(
            fen=d["fen"],
            side_to_move=d["side_to_move"],
            candidates=cands,
            objective_best=cands[d["objective_best"]],
            practical_best=cands[d["practical_best"]],
            edge=d["edge"],
            clear=d["clear"],
            params=PracticalParams.from_dict(d["params"]),
            white_elo=d["white_elo"],
            black_elo=d["black_elo"],
            leaf_model=d.get("leaf_model"),
        )

    @property
    def recommended(self) -> Candidate:
        """The move to play: the practical best if its edge is clear, else the objective best."""
        return self.practical_best if self.clear else self.objective_best

    def summary(self) -> str:
        """The verdict in words, as the app and ``trickiness practical --summary`` give it."""
        ob, pb = self.objective_best, self.practical_best
        opp, opp_elo = ("Black", self.black_elo) if self.side_to_move == "white" else ("White", self.white_elo)
        lines = [f"Objective best: {ob.san} ({ob.objective:.2f})."]
        if pb.uci == ob.uci:
            lines.append(f"Practical best: the same move ({pb.practical:.2f} against this opponent).")
        elif not self.clear:
            lines.append(
                f"Practical best: no clear edge over {ob.san} "
                f"({pb.san} gains only {self.edge:+.3f}; the margin is {self.params.margin})."
            )
        else:
            lines.append(
                f"Practical best: {pb.san} ({pb.practical:.2f}, {self.edge:+.2f} over {ob.san} against this opponent)."
            )
            if pb.best_reply:
                lines.append(
                    f"Relies on: {opp} missing {pb.best_reply}, which {opp_elo:.0f}s miss "
                    f"{pb.miss_chance:.0%} of the time. If they find it: {pb.objective:.2f}."
                )
        return "\n".join(lines)

    def to_frame(self) -> pd.DataFrame:
        """One row per candidate move, best practical value first. Needs pandas."""
        pandas = _require_pandas()
        df: pd.DataFrame = pandas.DataFrame(
            [
                {
                    "move": c.san,
                    "objective": c.objective,
                    "practical": c.practical,
                    "gain": c.practical - c.objective,
                    "miss_chance": c.miss_chance,
                    "best_reply": c.best_reply,
                    "maia_p": c.p,
                    "engine_rank": c.engine_rank,
                }
                for c in self.candidates
            ]
        ).set_index("move")
        # Moves outside Stockfish's MultiPV have no rank: a nullable integer, not float NaN.
        df["engine_rank"] = df["engine_rank"].astype("Int64")
        return df
