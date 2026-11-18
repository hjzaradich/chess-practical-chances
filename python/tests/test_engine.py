"""The bindings against real Stockfish and Maia. Skipped if either is missing."""

from __future__ import annotations

import threading
from pathlib import Path

import pytest

import practical_chances as pc

pytestmark = pytest.mark.engines


def test_engine_identity(engine: pc.Engine) -> None:
    assert engine.stockfish_name.startswith("Stockfish")
    assert engine.maia_id.startswith("maia3-5m.history.fp32.onnx@")
    assert engine.nodes == 20_000
    assert "Engine(" in repr(engine)


def test_analyze_position_matches_settings(engine: pc.Engine) -> None:
    r = engine.analyze_position("e4 e5 Nf3", white_elo=1400, black_elo=1600)
    assert r.ply == 3 and r.side_to_move == "black"
    assert r.settings.white_elo == 1400 and r.settings.black_elo == 1600
    assert r.settings.k == 1 and r.settings.nodes == 20_000  # the engine's preset and nodes
    assert r.settings.stockfish == engine.stockfish_name
    assert r.likely_moves and abs(sum(m.p for m in r.likely_moves) - 1) < 0.2


def test_moves_as_list_or_string_agree(engine: pc.Engine) -> None:
    a = engine.analyze_position("1. e4 e5 2. Nf3", white_elo=1500, black_elo=1500)
    b = engine.analyze_position(["e4", "e5", "Nf3"], white_elo=1500, black_elo=1500)
    assert a == b  # same position, cached evaluations: identical dataclasses


def test_analyze_game_streams_every_ply(engine: pc.Engine, miniature_pgn: Path) -> None:
    seen: list[int] = []
    g = engine.analyze_game(miniature_pgn, progress=lambda r: seen.append(r.ply))
    assert len(g) == 14 and seen == list(range(14))
    assert g.settings.white_elo == 1200  # from the PGN headers
    assert g[13].v_star == pytest.approx(1.0, abs=1e-3)  # White has mated


def test_callback_exception_stops_the_analysis(engine: pc.Engine, miniature_pgn: Path) -> None:
    class Stop(Exception):
        pass

    seen: list[int] = []

    def progress(r: pc.PlyResult) -> None:
        seen.append(r.ply)
        if r.ply == 2:
            raise Stop

    with pytest.raises(Stop):  # the callback's own exception, not an EngineError
        engine.analyze_game(miniature_pgn, progress=progress)
    assert seen == [0, 1, 2]


def test_practical_move(engine: pc.Engine) -> None:
    r = engine.practical_move("e4 e5 Bc4 Nc6", white_elo=800, black_elo=800)
    assert r.side_to_move == "white"
    assert {c.san for c in r.candidates} >= {"Nf3", "Qh5"}
    assert r.recommended in r.candidates


def test_bad_input_raises_value_error(engine: pc.Engine) -> None:
    with pytest.raises(ValueError, match="Ke3"):
        engine.analyze_position("e4 e5 Ke3", white_elo=1500, black_elo=1500)
    with pytest.raises(ValueError, match="leaf"):
        engine.practical_move("e4", leaf="magic")  # type: ignore[arg-type]
    with pytest.raises(ValueError, match="game 3 not found"):
        engine.analyze_game("1. e4 e5 *", game=2)


def test_threads_share_one_engine(engine: pc.Engine) -> None:
    lines = ["e4 c5", "d4 d5", "e4 e6", "c4 e5"]
    out: dict[str, float] = {}

    def run(m: str) -> None:
        out[m] = engine.analyze_position(m, white_elo=1500, black_elo=1500).v_star

    threads = [threading.Thread(target=run, args=(m,)) for m in lines]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert set(out) == set(lines)


def test_closed_engine_refuses_work(miniature_pgn: Path) -> None:
    try:
        local = Path(__file__).resolve().parents[2] / "engines" / "stockfish.exe"
        sf = pc.find_stockfish(local if local.is_file() else None)
        model = pc.find_maia_model()
    except pc.NotFoundError as e:
        pytest.skip(str(e))
    eng = pc.Engine(sf, model, preset="quick", nodes=5_000, workers=1, cache=False)
    eng.close()
    eng.close()  # twice is fine
    assert "closed" in repr(eng)
    with pytest.raises(pc.EngineError, match="closed"):
        eng.analyze_position("e4", white_elo=1500, black_elo=1500)
