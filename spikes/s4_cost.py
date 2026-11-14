"""Spike 4: cost of analysing one game. A throwaway version of the T_X tree from SPEC.md,
evaluated level by level so Maia is batched and Stockfish searches run in a process pool.

Per ply it does: one root Stockfish search (MultiPV 5, gives v* and stability), plus the
T_White and T_Black trees. Counts tree positions and Stockfish/Maia calls, with and without
the transposition cache, and the wall time.

Run: .venv/Scripts/python s4_cost.py --k 2 --p 0.9 --nodes 50000 [--plies 80] [--workers 4]
"""
import argparse
import json
import queue
import statistics
import threading
import time
from concurrent.futures import ThreadPoolExecutor

import chess
import numpy as np

from common import MODELS, MaiaOnnx, Stockfish, expected_score, HISTORY

ap = argparse.ArgumentParser()
ap.add_argument("--k", type=int, default=2)
ap.add_argument("--p", type=float, default=0.9)
ap.add_argument("--nodes", type=int, default=50_000)
ap.add_argument("--plies", type=int, default=80)
ap.add_argument("--workers", type=int, default=4)
ap.add_argument("--elo", type=int, default=1500)
ap.add_argument("--no-cache", action="store_true")
ap.add_argument("--out", default=None)
ap.add_argument("--game", default="maia", choices=["maia", "kasparov"])
args = ap.parse_args()
D = 0.5

maia = MaiaOnnx(MODELS / "maia3-5m.history.fp32.onnx", history_input=True, threads=1)


# ---------------------------------------------------------------- test game
def make_game(n_plies, seed=7):
    """An 80-ply human-like game: Maia-1500 self-play, sampled. Deterministic."""
    rng = np.random.default_rng(seed)
    hist = [chess.Board()]
    while len(hist) <= n_plies and not hist[-1].is_game_over():
        o = maia.evaluate([hist], 1500, 1500)[0]
        m = o.moves[rng.choice(len(o.moves), p=o.probs / o.probs.sum())]
        b = hist[-1].copy(stack=False); b.push(m); hist.append(b)
    return hist


KASPAROV_TOPALOV_1999 = (
    "e4 d6 d4 Nf6 Nc3 g6 Be3 Bg7 Qd2 c6 f3 b5 Nge2 Nbd7 Bh6 Bxh6 Qxh6 Bb7 a3 e5 O-O-O Qe7 "
    "Kb1 a6 Nc1 O-O-O Nb3 exd4 Rxd4 c5 Rd1 Nb6 g3 Kb8 Na5 Ba8 Bh3 d5 Qf4+ Ka7 Rhe1 d4 "
    "Nd5 Nbxd5 exd5 Qd6 Rxd4 cxd4 Re7+ Kb6 Qxd4+ Kxa5 b4+ Ka4 Qc3 Qxd5 Ra7 Bb7 Rxb7 Qc4 "
    "Qxf6 Kxa3 Qxa6+ Kxb4 c3+ Kxc3 Qa1+ Kd2 Qb2+ Kd1 Bf1 Rd2 Rd7 Rxd7 Bxc4 bxc4 Qxh8 Rd3 "
    "Qa8 c3 Qa4+ Ke1 f4 f5 Kc1 Rd2 Qa7")


def game_from_san(sans):
    hist = [chess.Board()]
    for s in sans.split():
        b = hist[-1].copy(stack=False); b.push_san(s); hist.append(b)
    return hist


GAME = make_game(args.plies) if args.game == "maia" else game_from_san(KASPAROV_TOPALOV_1999)[:args.plies + 1]
maia.calls = maia.positions = 0


# ---------------------------------------------------------------- engines + caches
pool: "queue.Queue[Stockfish]" = queue.Queue()
for _ in range(args.workers):
    pool.put(Stockfish(threads=1, hash_mb=32))
executor = ThreadPoolExecutor(args.workers)
sf_cache, maia_cache = {}, {}
stats = dict(sf_requests=0, sf_searches=0, maia_requests=0, maia_positions=0, nodes=0, terminal=0)
sf_lock = threading.Lock()


def fen_key(b: chess.Board) -> str:
    return " ".join(b.fen().split()[:4])


def sf_search(fen, multipv=1):
    s = pool.get()
    try:
        return s.search(fen, args.nodes, multipv=multipv)
    finally:
        pool.put(s)


def sf_batch(boards, multipv=1):
    """Returns [(wdl, bestmove_uci)] with caching; uncached ones run in parallel."""
    keys = [(fen_key(b), multipv) for b in boards]
    stats["sf_requests"] += len(boards)
    todo = {}
    for k, b in zip(keys, boards):
        if (args.no_cache or k not in sf_cache) and k not in todo:
            todo[k] = b
    results = list(executor.map(lambda b: sf_search(b.fen(), multipv), todo.values()))
    stats["sf_searches"] += len(todo)
    for k, r in zip(todo, results):
        sf_cache[k] = r
    return [sf_cache[k] for k in keys]


def maia_batch(hists, elo_self, elo_oppo):
    stats["maia_requests"] += len(hists)
    keys = [(tuple(fen_key(b) for b in h[-HISTORY:]), elo_self, elo_oppo) for h in hists]
    todo = {}
    for k, h in zip(keys, hists):
        if (args.no_cache or k not in maia_cache) and k not in todo:
            todo[k] = h
    if todo:
        outs = maia.evaluate(list(todo.values()), elo_self, elo_oppo)
        stats["maia_positions"] += len(todo)
        for k, o in zip(todo, outs):
            maia_cache[k] = o
    return [maia_cache[k] for k in keys]


# ---------------------------------------------------------------- tree
class Node:
    __slots__ = ("hist", "X", "dl", "kind", "children", "value", "leaf_v")

    def __init__(self, hist, X, dl):
        self.hist, self.X, self.dl = hist, X, dl
        self.children, self.value, self.leaf_v = [], None, None
        b = hist[-1]
        stats["nodes"] += 1
        if b.is_game_over(claim_draw=False):
            stats["terminal"] += 1
            self.kind = "term"
            res = b.result(claim_draw=False)
            if res == "1/2-1/2":
                self.value = D
            else:
                self.value = 1.0 if (res == "1-0") == (X == chess.WHITE) else 0.0
        elif b.turn == X:
            self.kind = "x"
        else:
            self.kind = "opp"


def child(hist, move, X, dl):
    b = hist[-1].copy(stack=False)
    b.push(move)
    return Node((hist + [b])[-HISTORY:], X, dl)


def value_for(X, side_to_move, wdl):
    w, d, l = wdl
    return expected_score((w, d, l), D) if side_to_move == X else expected_score((l, d, w), D)


def build_and_eval(roots, elos, p):
    """roots: list of Node. Expands level by level, then evaluates bottom-up."""
    frontier = list(roots)
    while frontier:
        xs = [n for n in frontier if n.kind == "x"]
        opps = [n for n in frontier if n.kind == "opp"]
        nxt = []
        # Maia, batched per (rating pair)
        for X in (chess.WHITE, chess.BLACK):
            grp = [n for n in xs if n.X == X]
            if not grp:
                continue
            outs = maia_batch([n.hist for n in grp], elos[X], elos[not X])
            for n, o in zip(grp, outs):
                cum, chosen = 0.0, []
                for m, pr in zip(o.moves, o.probs):
                    chosen.append((m, pr)); cum += pr
                    if cum >= p:
                        break
                tot = sum(pr for _, pr in chosen)
                for m, pr in chosen:
                    c = child(n.hist, m, n.X, n.dl - 1)
                    n.children.append((pr / tot, c)); nxt.append(c)
        # Stockfish for opponent nodes (value, and best move if not a leaf)
        if opps:
            res = sf_batch([n.hist[-1] for n in opps])
            for n, r in zip(opps, res):
                pv = r.pvs[0]
                n.leaf_v = value_for(n.X, n.hist[-1].turn, pv.wdl)
                if n.dl > 0:
                    c = child(n.hist, chess.Move.from_uci(pv.move), n.X, n.dl)
                    n.children.append((1.0, c)); nxt.append(c)
        frontier = nxt

    def ev(n):
        if n.value is not None:
            return n.value
        if n.kind == "opp":
            n.value = n.leaf_v if n.dl == 0 else ev(n.children[0][1])
        else:
            n.value = sum(w * ev(c) for w, c in n.children)
        return n.value

    return [ev(r) for r in roots]


# ---------------------------------------------------------------- run
elos = {chess.WHITE: args.elo, chess.BLACK: args.elo}
rows = []
per_ply_nodes = []
t_start = time.perf_counter()
for ply in range(len(GAME)):
    hist = GAME[max(0, ply - HISTORY + 1): ply + 1]
    b = hist[-1]
    if b.is_game_over():
        break
    n0 = stats["nodes"]
    root_res = sf_batch([b], multipv=5)[0]
    v_star_stm = expected_score(root_res.pvs[0].wdl, D)
    roots = [Node(hist, chess.WHITE, args.k), Node(hist, chess.BLACK, args.k)]
    vw, vb = build_and_eval(roots, elos, args.p)
    v_star_w = v_star_stm if b.turn == chess.WHITE else 1 - v_star_stm
    rows.append(dict(ply=ply, fen=b.fen(), v_star_white=round(v_star_w, 4),
                     T_white=round(v_star_w - vw, 4), T_black=round((1 - v_star_w) - vb, 4)))
    per_ply_nodes.append(stats["nodes"] - n0)
elapsed = time.perf_counter() - t_start

for _ in range(args.workers):
    pool.get().close()

summary = dict(k=args.k, p=args.p, nodes=args.nodes, workers=args.workers, plies=len(rows),
               cache=not args.no_cache, seconds=round(elapsed, 1),
               sec_per_ply=round(elapsed / len(rows), 2),
               tree_positions_per_ply_mean=round(statistics.mean(per_ply_nodes), 1),
               tree_positions_per_ply_max=max(per_ply_nodes),
               sf_requests=stats["sf_requests"], sf_searches_run=stats["sf_searches"],
               maia_requests=stats["maia_requests"], maia_positions_run=stats["maia_positions"],
               maia_batches=maia.calls)
print(json.dumps(summary))
if args.out:
    with open(args.out, "w") as f:
        json.dump(dict(summary=summary, rows=rows), f, indent=1)
