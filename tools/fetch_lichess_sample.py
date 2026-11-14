"""Stream the start of a Lichess monthly database file and keep games for Phase 2.

Keeps rated Blitz/Rapid/Classical games (base time >= 3 min) where *both* players are in a
rating band, ended normally or on time, with at least 20 plies. Stops downloading once every
band has enough games; only the first tens of MB of the ~30 GB file are read.

Each kept game gets a [SamplePly "t"] header: the one position per game used for the T test,
uniform over plies 6..(plies-1), seeded, so runs are reproducible.

    spikes/.venv/Scripts/python tools/fetch_lichess_sample.py --month 2026-10 --per-band 3000
"""
import argparse
import io
import random
import re
import sys
import urllib.request
from pathlib import Path

import zstandard

ap = argparse.ArgumentParser()
ap.add_argument("--month", default="2026-10")
ap.add_argument("--per-band", type=int, default=3000)
ap.add_argument("--bands", default="1400-1600,1900-2100")
ap.add_argument("--out", default="validation/data")
ap.add_argument("--seed", type=int, default=2026)
args = ap.parse_args()

bands = [tuple(int(x) for x in b.split("-")) for b in args.bands.split(",")]
out_dir = Path(args.out)
out_dir.mkdir(parents=True, exist_ok=True)
rng = random.Random(args.seed)
url = f"https://database.lichess.org/standard/lichess_db_standard_rated_{args.month}.pgn.zst"

TAG = re.compile(r'^\[(\w+) "(.*)"\]$', re.M)
MOVE_NUM = re.compile(r"\d+\.(\.\.)?")
COMMENT = re.compile(r"\{[^}]*\}")


def count_plies(movetext: str) -> int:
    t = COMMENT.sub(" ", movetext)
    t = MOVE_NUM.sub(" ", t)
    toks = [x for x in t.split() if x not in ("1-0", "0-1", "1/2-1/2", "*") and not x.startswith("$")]
    return len(toks)


kept = {b: [] for b in bands}
seen = 0
req = urllib.request.Request(url, headers={"User-Agent": "trickiness-validation/0.1"})
with urllib.request.urlopen(req) as resp:
    reader = zstandard.ZstdDecompressor().stream_reader(resp)
    text = io.TextIOWrapper(reader, encoding="utf-8", errors="replace")
    buf = []
    for line in text:
        if line.startswith("[Event ") and buf:
            game = "".join(buf)
            buf = [line]
            seen += 1
            head, _, moves = game.partition("\n\n")
            tags = dict(TAG.findall(head))
            ev = tags.get("Event", "")
            if not ev.startswith("Rated") or not any(k in ev for k in ("Blitz", "Rapid", "Classical")):
                continue
            tc = tags.get("TimeControl", "-")
            if "+" not in tc or int(tc.split("+")[0]) < 180:
                continue
            if tags.get("Termination") not in ("Normal", "Time forfeit"):
                continue
            try:
                we, be = int(tags["WhiteElo"]), int(tags["BlackElo"])
            except (KeyError, ValueError):
                continue
            for lo, hi in bands:
                if lo <= we <= hi and lo <= be <= hi and len(kept[(lo, hi)]) < args.per_band:
                    plies = count_plies(moves)
                    if plies >= 20:
                        t = rng.randint(6, plies - 1)
                        game = head + f'\n[SamplePly "{t}"]\n[PlyCount "{plies}"]\n\n' + moves.strip() + "\n\n"
                        kept[(lo, hi)].append(game)
            if seen % 20000 == 0:
                print(f"{seen} games read; kept " + ", ".join(f"{lo}-{hi}: {len(v)}" for (lo, hi), v in kept.items()),
                      file=sys.stderr, flush=True)
            if all(len(v) >= args.per_band for v in kept.values()):
                break
        else:
            buf.append(line)

for (lo, hi), games in kept.items():
    p = out_dir / f"band_{lo}_{hi}.pgn"
    p.write_text("".join(games), encoding="utf-8")
    print(f"{p}: {len(games)} games", file=sys.stderr)
print(f"read {seen} games from the start of {url}", file=sys.stderr)
