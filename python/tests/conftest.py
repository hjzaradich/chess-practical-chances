from __future__ import annotations

import json
from collections.abc import Iterator
from pathlib import Path
from typing import Any

import pytest

import practical_chances as pc

DATA = Path(__file__).parent / "data"
REPO = Path(__file__).resolve().parents[2]


@pytest.fixture
def miniature_pgn() -> Path:
    """Legall's mate (13 plies, WhiteElo 1200 / BlackElo 1100) and a second game with an illegal move."""
    return DATA / "miniature.pgn"


@pytest.fixture
def miniature_lines() -> list[str]:
    """`trickiness analyze miniature.pgn --preset quick`, one PlyResult JSON per line."""
    return (DATA / "miniature.quick.jsonl").read_text(encoding="utf-8").splitlines()


def _load(name: str) -> dict[str, Any]:
    data: dict[str, Any] = json.loads((DATA / name).read_text(encoding="utf-8"))
    return data


@pytest.fixture
def practical_clear() -> dict[str, Any]:
    """1.e4 e5 2.Bc4 Nc6 at 800: practical best Qh5, clearly ahead of the objective best Nf3."""
    return _load("bc4_800.practical.json")


@pytest.fixture
def practical_same() -> dict[str, Any]:
    """…3.Qh5 at 800, Black to move: practical best = objective best."""
    return _load("scholar_800.practical.json")


@pytest.fixture(scope="session")
def engine() -> Iterator[pc.Engine]:
    """A real engine, small and memory-cached. Skips the test if Stockfish or the model is missing."""
    local_sf = REPO / "engines" / "stockfish.exe"
    try:
        sf = pc.find_stockfish(local_sf if local_sf.is_file() else None)
        model = pc.find_maia_model()
    except pc.NotFoundError as e:
        pytest.skip(str(e))
    with pc.Engine(sf, model, preset="quick", nodes=20_000, workers=2, cache=False) as eng:
        yield eng
