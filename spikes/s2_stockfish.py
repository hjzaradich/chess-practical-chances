"""Spike 2: Stockfish over UCI. Confirms MultiPV + UCI_ShowWDL output and times searches
at several node limits on this machine.

Run: .venv/Scripts/python s2_stockfish.py
"""
import statistics
import time

import chess

from common import STOCKFISH, Stockfish, boards_from_moves, expected_score
from positions import SANITY, RUY

NODE_LIMITS = [10_000, 50_000, 200_000, 1_000_000]

positions = [(n, boards_from_moves(f, m)[-1].fen()) for n, f, m in SANITY]
positions.append(("ruy_closed", boards_from_moves(None, RUY)[-1].fen()))

sf = Stockfish(threads=1, hash_mb=64)
sf._send("uci")
ident = [l for l in sf._wait("uciok") if l.startswith("id name")]
print(ident[0], "|", STOCKFISH.name)

# ---- raw output sample
sf.setoption("MultiPV", 3); sf.multipv = 3
sf._send(f"position fen {positions[2][1]}")
sf._send("go nodes 50000")
lines = sf._wait("bestmove")
print("\nraw output, MultiPV 3, Najdorf, last lines:")
for ln in [l for l in lines if " pv " in l][-3:]:
    print("  ", ln[:160])
print("  ", lines[-1])

# ---- WDL + MultiPV per position
print("\nMultiPV 5 at 200k nodes, WDL per mille (side to move), v = W + D/2:")
for name, fen in positions:
    sf.newgame()
    r = sf.search(fen, 200_000, multipv=5)
    s = "  ".join(f"{chess.Board(fen).san(chess.Move.from_uci(p.move))}:{p.wdl[0]}/{p.wdl[1]}/{p.wdl[2]}"
                  for p in r.pvs)
    print(f"  {name:28s} d={r.pvs[0].depth:2d}  {s}")

# ---- timing per node limit, single thread, MultiPV 1 and 5
print("\ntiming (1 thread, fresh hash per position):")
print(f"  {'nodes':>9s} {'mpv':>3s} {'ms/pos median':>14s} {'ms/pos max':>11s} {'Mnps':>6s} {'depth':>5s}")
for mpv in (1, 5):
    for nl in NODE_LIMITS:
        times, nps, depth = [], [], []
        for name, fen in positions:
            sf.newgame()
            r = sf.search(fen, nl, multipv=mpv)
            times.append(r.seconds * 1000)
            nps.append(r.nps)
            depth.append(r.pvs[0].depth)
        print(f"  {nl:9d} {mpv:3d} {statistics.median(times):14.1f} {max(times):11.1f} "
              f"{statistics.median(nps)/1e6:6.2f} {statistics.median(depth):5.0f}")

# ---- eval stability: how much does v move between node limits (noise floor for T)
print("\nv (expected score, side to move) by node limit:")
for name, fen in positions:
    vs = []
    for nl in NODE_LIMITS:
        sf.newgame()
        vs.append(expected_score(sf.search(fen, nl).pvs[0].wdl))
    print(f"  {name:28s} " + " ".join(f"{v:.3f}" for v in vs))
sf.close()

# ---- parallel throughput: N single-thread processes
import concurrent.futures as cf


def worker(args):
    nl, fens = args
    s = Stockfish(threads=1, hash_mb=32)
    t0 = time.perf_counter()
    for fen in fens:
        s.search(fen, nl)
    dt = time.perf_counter() - t0
    s.close()
    return dt


print("\nparallel throughput at 200k nodes (positions/s across processes):")
fens = [f for _, f in positions] * 3
for procs in (1, 2, 4, 8):
    t0 = time.perf_counter()
    with cf.ThreadPoolExecutor(procs) as ex:
        list(ex.map(worker, [(200_000, fens)] * procs))
    dt = time.perf_counter() - t0
    print(f"  {procs} processes: {procs*len(fens)/dt:6.1f} pos/s")
