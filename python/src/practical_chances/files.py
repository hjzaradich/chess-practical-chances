"""Finding Stockfish and the Maia model, and downloading the model.

Lookup order, for both: an explicit argument, then an environment variable
(``TRICKINESS_STOCKFISH`` / ``TRICKINESS_MAIA``, the same ones the CLI and app read), then
the usual places. The model download is the GitHub release the desktop app uses, checked
against the same SHA-256 (DECISIONS 5.1).
"""

from __future__ import annotations

import hashlib
import os
import shutil
import sys
import tempfile
import urllib.request
from collections.abc import Callable
from pathlib import Path

MODEL_FILE = "maia3-5m.history.fp32.onnx"
MODEL_URL = f"https://github.com/hjzaradich/chess-practical-chances/releases/download/maia3-5m-history-v1/{MODEL_FILE}"
MODEL_SHA256 = "f2fc76df6749947d0325fa71e27a87fa0f065abd09b04b8ec01006571040e8a7"

StrPath = str | os.PathLike[str]


class NotFoundError(FileNotFoundError):
    """Stockfish or the model isn't where we looked. The message says where that was."""


def user_cache_dir() -> Path:
    """Per-user folder for the model and the evaluation cache (``$PRACTICAL_CHANCES_HOME`` overrides)."""
    if env := os.environ.get("PRACTICAL_CHANCES_HOME"):
        return Path(env)
    if sys.platform == "win32":
        base = Path(os.environ.get("LOCALAPPDATA") or Path.home() / "AppData" / "Local")
    elif sys.platform == "darwin":
        base = Path.home() / "Library" / "Caches"
    else:
        base = Path(os.environ.get("XDG_CACHE_HOME") or Path.home() / ".cache")
    return base / "practical-chances"


def _app_model() -> Path | None:
    """The desktop app's copy of the model, if it's installed (no need to download twice)."""
    if sys.platform == "win32" and (appdata := os.environ.get("APPDATA")):
        return Path(appdata) / "app.trickiness.desktop" / "models" / MODEL_FILE
    return None


def _first(candidates: list[Path | None]) -> Path | None:
    return next((p for p in candidates if p is not None and p.is_file()), None)


def find_stockfish(path: StrPath | None = None) -> Path:
    """Stockfish: ``path``, ``$TRICKINESS_STOCKFISH``, ``engines/`` here, or ``stockfish`` on PATH."""
    exe = "stockfish.exe" if sys.platform == "win32" else "stockfish"
    if path is not None:
        if not Path(path).is_file():
            raise NotFoundError(f"no Stockfish at {path}")
        return Path(path)
    env = os.environ.get("TRICKINESS_STOCKFISH")
    on_path = shutil.which("stockfish")
    found = _first([Path(env) if env else None, Path("engines") / exe, Path(on_path) if on_path else None])
    if found is None:
        raise NotFoundError(
            "Stockfish not found. Install it (https://stockfishchess.org/download/), put it on PATH, "
            "set TRICKINESS_STOCKFISH, or pass stockfish=..."
        )
    return found


def find_maia_model(path: StrPath | None = None) -> Path:
    """The Maia model: ``path``, ``$TRICKINESS_MAIA``, the download cache, the app's copy, or ``models/`` here."""
    if path is not None:
        if not Path(path).is_file():
            raise NotFoundError(f"no model file at {path}")
        return Path(path)
    env = os.environ.get("TRICKINESS_MAIA")
    found = _first(
        [
            Path(env) if env else None,
            user_cache_dir() / MODEL_FILE,
            _app_model(),
            Path("models") / MODEL_FILE,
        ]
    )
    if found is None:
        raise NotFoundError(
            "The Maia model isn't downloaded yet. Run practical_chances.download_model() "
            "(21 MB, once), set TRICKINESS_MAIA, or pass maia_model=..."
        )
    return found


def download_model(
    dest: StrPath | None = None,
    *,
    url: str = MODEL_URL,
    sha256: str = MODEL_SHA256,
    progress: Callable[[int, int], None] | None = None,
) -> Path:
    """Download the Maia model and check its SHA-256. Returns its path.

    Skips the download if a file with the right checksum is already there. The file is written
    under a temporary name and only moved into place once the checksum matches, so an
    interrupted or corrupted download never leaves a bad model behind. ``progress`` is called
    with (bytes done, total bytes; 0 if unknown).
    """
    target = Path(dest) if dest is not None else user_cache_dir() / MODEL_FILE
    if target.is_file() and _sha256(target) == sha256:
        return target
    target.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp_name = tempfile.mkstemp(dir=target.parent, prefix=target.name, suffix=".part")
    tmp = Path(tmp_name)
    try:
        h = hashlib.sha256()
        with os.fdopen(fd, "wb") as out, urllib.request.urlopen(url, timeout=60) as resp:
            total = int(resp.headers.get("Content-Length") or 0)
            done = 0
            while chunk := resp.read(1 << 16):
                out.write(chunk)
                h.update(chunk)
                done += len(chunk)
                if progress:
                    progress(done, total)
        if (got := h.hexdigest()) != sha256:
            raise ValueError(f"checksum mismatch for {url}: expected {sha256[:12]}…, got {got[:12]}…")
        tmp.replace(target)
    finally:
        tmp.unlink(missing_ok=True)
    return target


def _sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        while chunk := f.read(1 << 20):
            h.update(chunk)
    return h.hexdigest()
