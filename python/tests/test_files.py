"""Finding and downloading the engines (no network: downloads use file:// URLs)."""

from __future__ import annotations

import hashlib
from pathlib import Path

import pytest

import practical_chances as pc
from practical_chances import files


@pytest.fixture
def fake_model(tmp_path: Path) -> tuple[str, str]:
    src = tmp_path / "source.onnx"
    src.write_bytes(b"not really a model" * 1000)
    return src.as_uri(), hashlib.sha256(src.read_bytes()).hexdigest()


def test_download_checks_the_checksum(tmp_path: Path, fake_model: tuple[str, str]) -> None:
    url, sha = fake_model
    seen: list[tuple[int, int]] = []
    dest = pc.download_model(
        tmp_path / "out" / "m.onnx", url=url, sha256=sha, progress=lambda d, t: seen.append((d, t))
    )
    assert dest.read_bytes() == b"not really a model" * 1000
    assert seen and seen[-1][0] == 18_000


def test_bad_checksum_leaves_nothing_behind(tmp_path: Path, fake_model: tuple[str, str]) -> None:
    url, _ = fake_model
    out = tmp_path / "out"
    with pytest.raises(ValueError, match="checksum mismatch"):
        pc.download_model(out / "m.onnx", url=url, sha256="0" * 64)
    assert list(out.iterdir()) == []  # no model and no .part file


def test_existing_good_file_is_not_downloaded_again(tmp_path: Path, fake_model: tuple[str, str]) -> None:
    url, sha = fake_model
    dest = pc.download_model(tmp_path / "m.onnx", url=url, sha256=sha)
    mtime = dest.stat().st_mtime_ns
    assert pc.download_model(dest, url="file:///nowhere", sha256=sha) == dest
    assert dest.stat().st_mtime_ns == mtime


def test_env_vars_and_explicit_paths(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    sf = tmp_path / "sf"
    sf.write_text("")
    model = tmp_path / "m.onnx"
    model.write_text("")
    monkeypatch.setenv("TRICKINESS_STOCKFISH", str(sf))
    monkeypatch.setenv("TRICKINESS_MAIA", str(model))
    assert pc.find_stockfish() == sf
    assert pc.find_maia_model() == model
    with pytest.raises(pc.NotFoundError):
        pc.find_stockfish(tmp_path / "missing")


def test_not_found_says_what_to_do(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.chdir(tmp_path)
    monkeypatch.delenv("TRICKINESS_MAIA", raising=False)
    monkeypatch.setenv("PRACTICAL_CHANCES_HOME", str(tmp_path / "home"))
    monkeypatch.setattr(files, "_app_model", lambda: None)
    with pytest.raises(pc.NotFoundError, match="download_model"):
        pc.find_maia_model()


def test_cache_dir_override(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("PRACTICAL_CHANCES_HOME", str(tmp_path))
    assert pc.user_cache_dir() == tmp_path
