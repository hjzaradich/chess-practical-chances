"""PGN parsing through the Rust core (no engines needed)."""

from __future__ import annotations

from pathlib import Path

import practical_chances as pc


def test_parse_from_path_and_text_agree(miniature_pgn: Path) -> None:
    from_path = pc.parse_pgn(miniature_pgn)
    from_text = pc.parse_pgn(miniature_pgn.read_text(encoding="utf-8"))
    assert from_path == from_text


def test_games_tags_and_moves(miniature_pgn: Path) -> None:
    games = pc.parse_pgn(miniature_pgn)
    assert len(games) == 2
    g = games[0]
    assert g.error is None
    assert g.tags["Event"] == "Legall's mate (fixture)"
    assert g.elo("white") == 1200 and g.elo("black") == 1100
    assert g.sans[:4] == ["e4", "e5", "Nf3", "d6"] and g.sans[-1] == "Nd5#"
    assert g.moves[0].uci == "e2e4"
    assert g.moves[0].fen.startswith("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b")
    assert g.start_fen is not None and g.start_fen.startswith("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w")


def test_a_bad_game_keeps_its_place(miniature_pgn: Path) -> None:
    bad = pc.parse_pgn(miniature_pgn)[1]
    assert bad.index == 1
    assert bad.error is not None and "Ke3" in bad.error
    assert bad.moves == ()


def test_missing_elo_is_none() -> None:
    (g,) = pc.parse_pgn('[White "A"]\n[WhiteElo "?"]\n\n1. d4 *\n')
    assert g.elo("white") is None and g.elo("black") is None


def test_fen_start() -> None:
    (g,) = pc.parse_pgn('[FEN "8/8/8/8/8/8/k7/2K4R w - - 0 1"]\n[SetUp "1"]\n\n1. Rh2+ Ka1 *\n')
    assert g.start_fen == "8/8/8/8/8/8/k7/2K4R w - - 0 1"
    assert g.sans == ["Rh2+", "Ka1"]
