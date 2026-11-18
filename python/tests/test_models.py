"""The dataclasses against saved core output (no engines needed)."""

from __future__ import annotations

import dataclasses
from typing import Any

import pandas as pd
import pytest

import practical_chances as pc


def test_game_analysis_parses_every_ply(miniature_lines: list[str]) -> None:
    g = pc.GameAnalysis.from_json_lines(miniature_lines)
    assert len(g) == 14  # the start plus 13 moves
    assert [r.ply for r in g] == list(range(14))
    assert g[0].san is None and g[1].san == "e4" and g[13].san == "Nd5#"
    assert g.settings.k == 1  # the quick preset
    assert g.settings.white_elo == 1200 and g.settings.black_elo == 1100
    assert g.settings.maia.startswith("maia3-5m.history.fp32.onnx@")


def test_ply_values_are_in_range(miniature_lines: list[str]) -> None:
    for r in pc.GameAnalysis.from_json_lines(miniature_lines):
        assert 0.0 <= r.v_star <= 1.0
        assert 0.0 <= r.draw_rate <= 1.0
        assert r.side_to_move in ("white", "black")
        for m in r.likely_moves:
            assert 0.0 <= m.p <= 1.0


def test_game_over_has_no_diagnostics(miniature_lines: list[str]) -> None:
    mate = pc.GameAnalysis.from_json_lines(miniature_lines)[13]
    assert mate.diagnostics.white is None and mate.diagnostics.black is None
    assert tuple(mate.trickiness) == (0.0, 0.0)


def test_diagnostics_for_a_later_decision(miniature_lines: list[str]) -> None:
    r = pc.GameAnalysis.from_json_lines(miniature_lines)[3]  # after 2.Nf3, Black to move
    white = r.diagnostics["white"]
    assert white is not None
    assert white.at_ply_offset == 1 and white.via is not None  # White decides after Black's reply
    black = r.diagnostics["black"]
    assert black is not None and black.at_ply_offset == 0 and black.via is None


def test_results_are_frozen(miniature_lines: list[str]) -> None:
    r = pc.GameAnalysis.from_json_lines(miniature_lines)[1]
    with pytest.raises(dataclasses.FrozenInstanceError):
        r.v_star = 0.0  # type: ignore[misc]


def test_sides_indexing() -> None:
    s = pc.Sides(1, 2)
    assert s["white"] == 1 and s["black"] == 2 and list(s) == [1, 2]
    with pytest.raises(KeyError):
        s["red"]  # type: ignore[index]


def test_to_frame(miniature_lines: list[str]) -> None:
    df = pc.GameAnalysis.from_json_lines(miniature_lines).to_frame()
    assert isinstance(df, pd.DataFrame)
    assert df.index.name == "ply" and len(df) == 14
    assert {"move", "v_star", "draw_rate", "t_white", "t_black", "trap_mass_white"} <= set(df.columns)
    assert df.loc[13, "move"] == "Nd5#"
    assert df["trap_mass_white"].isna()[13]  # nobody moves after mate


def test_trickiest_is_sorted(miniature_lines: list[str]) -> None:
    g = pc.GameAnalysis.from_json_lines(miniature_lines)
    top = g.trickiest("white", n=3)
    values = [r.trickiness.white for r in top]
    assert values == sorted(values, reverse=True)
    assert values[0] == max(r.trickiness.white for r in g)


def test_practical_with_a_clear_edge(practical_clear: dict[str, Any]) -> None:
    r = pc.Practical.from_dict(practical_clear)
    assert r.side_to_move == "white"
    assert r.objective_best.san == "Nf3"
    assert r.practical_best.san == "Qh5"
    assert r.clear and r.edge >= r.params.margin
    assert r.recommended is r.practical_best
    assert r.practical_best.best_reply == "g6"
    assert r.edge == pytest.approx(r.practical_best.practical - r.objective_best.practical)
    # candidates come sorted by practical value
    q = [c.practical for c in r.candidates]
    assert q == sorted(q, reverse=True)


def test_practical_summary_wording(practical_clear: dict[str, Any], practical_same: dict[str, Any]) -> None:
    clear = pc.Practical.from_dict(practical_clear).summary()
    assert "Practical best: Qh5" in clear
    assert "Relies on: Black missing g6, which 800s miss" in clear
    same = pc.Practical.from_dict(practical_same).summary()
    assert "Practical best: the same move" in same


def test_practical_params_rename_lambda(practical_clear: dict[str, Any]) -> None:
    params = pc.Practical.from_dict(practical_clear).params
    assert params.lambda_ == practical_clear["params"]["lambda"]
    assert params.leaf == "calibrated"


def test_practical_to_frame(practical_clear: dict[str, Any]) -> None:
    df = pc.Practical.from_dict(practical_clear).to_frame()
    assert df.index[0] == "Qh5"
    assert str(df["engine_rank"].dtype) == "Int64"  # missing ranks are <NA>, not float NaN
    assert ((df["practical"] - df["objective"]) - df["gain"]).abs().max() < 1e-12
