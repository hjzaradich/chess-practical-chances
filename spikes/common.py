"""Shared helpers for the Phase 0 spikes. Throwaway code: not the app core.

- Maia-3 input encoding (copied from the official CSSLab/maia3 package, numpy version)
- A Maia ONNX wrapper that takes real move history
- A minimal Stockfish UCI wrapper with MultiPV + WDL
"""
from __future__ import annotations

import os
import subprocess
import time
from collections import deque
from dataclasses import dataclass, field
from pathlib import Path

import chess
import numpy as np

SPIKES = Path(__file__).resolve().parent
DL = SPIKES / "_dl"
MODELS = DL / "models"
STOCKFISH = DL / "stockfish" / "stockfish" / "stockfish-windows-x86-64-universal.exe"
HISTORY = 8


# ---------------------------------------------------------------- Maia encoding

PIECE_MAP = {chess.PAWN: 1, chess.KNIGHT: 2, chess.BISHOP: 3,
             chess.ROOK: 4, chess.QUEEN: 5, chess.KING: 6}


def all_moves() -> list[str]:
    moves = []
    for rank in range(8):
        for file in range(8):
            sq = chess.square(file, rank)
            for tr in range(8):
                for tf in range(8):
                    moves.append(chess.square_name(sq) + chess.square_name(chess.square(tf, tr)))
    for ff in "abcdefgh":
        for tf in "abcdefgh":
            for piece in "qrbn":
                moves.append(f"{ff}7{tf}8{piece}")
    return moves


ALL_MOVES = all_moves()
MOVE_INDEX = {m: i for i, m in enumerate(ALL_MOVES)}
assert len(ALL_MOVES) == 4352


def mirror_uci(u: str) -> str:
    return u[0] + str(9 - int(u[1])) + u[2] + str(9 - int(u[3])) + u[4:]


def tokenize(board: chess.Board) -> np.ndarray:
    """(64, 12) one-hot, board mirrored when black is to move (each slot by its own mover)."""
    t = np.zeros((64, 12), dtype=np.float32)
    b = board.mirror() if board.turn == chess.BLACK else board
    for sq, piece in b.piece_map().items():
        t[sq, PIECE_MAP[piece.piece_type] - 1 + (6 if piece.color == chess.BLACK else 0)] = 1
    return t


def history_tokens(boards: list[chess.Board]) -> np.ndarray:
    """boards: oldest..current (any length >= 1). Returns (64, 96), padded at the front
    with the earliest position, exactly as maia3.dataset.get_historical_tokens does."""
    boards = boards[-HISTORY:]
    toks = [tokenize(b) for b in boards]
    toks = [toks[0]] * (HISTORY - len(toks)) + toks
    return np.concatenate(toks, axis=1)


def boards_from_moves(start_fen: str | None, moves: list[str]) -> list[chess.Board]:
    b = chess.Board(start_fen) if start_fen else chess.Board()
    out = [b.copy(stack=False)]
    for u in moves:
        b.push_uci(u)
        out.append(b.copy(stack=False))
    return out


def legal_index(board: chess.Board) -> tuple[list[chess.Move], np.ndarray]:
    moves = list(board.legal_moves)
    idx = []
    for m in moves:
        u = m.uci()
        if board.turn == chess.BLACK:
            u = mirror_uci(u)
        idx.append(MOVE_INDEX[u])
    return moves, np.array(idx, dtype=np.int64)


def softmax(x: np.ndarray) -> np.ndarray:
    x = x - x.max()
    e = np.exp(x)
    return e / e.sum()


@dataclass
class MaiaOut:
    moves: list[chess.Move]          # legal moves, sorted by probability (desc)
    probs: np.ndarray                # matching probabilities
    wdl: tuple[float, float, float]  # side-to-move win, draw, loss


class MaiaOnnx:
    """Maia-3 via ONNX Runtime. `history_input=True` for our own export (history [B,64,96]);
    False for the community export (tokens [B,64,12], history replicated internally)."""

    def __init__(self, path: Path, history_input: bool = True, threads: int = 1):
        import onnxruntime as ort
        so = ort.SessionOptions()
        so.intra_op_num_threads = threads
        so.inter_op_num_threads = 1
        self.sess = ort.InferenceSession(str(path), so, providers=["CPUExecutionProvider"])
        self.history_input = history_input
        self.calls = 0
        self.positions = 0

    def raw(self, toks: np.ndarray, elo_self: np.ndarray, elo_oppo: np.ndarray):
        self.calls += 1
        self.positions += len(toks)
        if self.history_input:
            feeds = {"history": toks, "elo_self": elo_self, "elo_oppo": elo_oppo}
        else:
            feeds = {"tokens": toks, "elo_self": elo_self, "elo_oppo": elo_oppo}
        return self.sess.run(["logits_move", "logits_value"], feeds)

    def evaluate(self, histories: list[list[chess.Board]], elo_self: float, elo_oppo: float) -> list[MaiaOut]:
        if self.history_input:
            toks = np.stack([history_tokens(h) for h in histories])
        else:
            toks = np.stack([tokenize(h[-1]) for h in histories])
        n = len(histories)
        lm, lv = self.raw(toks, np.full(n, elo_self, np.float32), np.full(n, elo_oppo, np.float32))
        out = []
        for i, h in enumerate(histories):
            board = h[-1]
            moves, idx = legal_index(board)
            if not moves:
                p = np.zeros(0)
            else:
                p = softmax(lm[i][idx].astype(np.float64))
            order = np.argsort(-p, kind="stable")
            loss, draw, win = softmax(lv[i].astype(np.float64))
            out.append(MaiaOut([moves[j] for j in order], p[order], (win, draw, loss)))
        return out


# ---------------------------------------------------------------- Stockfish

@dataclass
class PV:
    move: str
    wdl: tuple[int, int, int]   # permille, side to move
    cp: int | None
    mate: int | None
    depth: int


@dataclass
class SearchResult:
    pvs: list[PV]
    nodes: int
    seconds: float
    nps: int


class Stockfish:
    def __init__(self, threads: int = 1, hash_mb: int = 64, path: Path = STOCKFISH):
        self.p = subprocess.Popen([str(path)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  text=True, bufsize=1, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        self._send("uci")
        self._wait("uciok")
        self.version = None
        self.setoption("Threads", threads)
        self.setoption("Hash", hash_mb)
        self.setoption("UCI_ShowWDL", "true")
        self.multipv = 1
        self.isready()

    def _send(self, s: str):
        self.p.stdin.write(s + "\n")
        self.p.stdin.flush()

    def _wait(self, token: str) -> list[str]:
        lines = []
        while True:
            line = self.p.stdout.readline()
            if not line:
                raise RuntimeError("stockfish died")
            line = line.strip()
            lines.append(line)
            if line.startswith(token):
                return lines

    def setoption(self, name, value):
        self._send(f"setoption name {name} value {value}")

    def isready(self):
        self._send("isready")
        self._wait("readyok")

    def newgame(self):
        self._send("ucinewgame")
        self.isready()

    def search(self, fen: str, nodes: int, multipv: int = 1, moves: list[str] | None = None) -> SearchResult:
        if multipv != self.multipv:
            self.setoption("MultiPV", multipv)
            self.multipv = multipv
        pos = f"position fen {fen}" + (" moves " + " ".join(moves) if moves else "")
        self._send(pos)
        t0 = time.perf_counter()
        self._send(f"go nodes {nodes}")
        lines = self._wait("bestmove")
        dt = time.perf_counter() - t0
        best: dict[int, PV] = {}
        last_nodes = 0
        for ln in lines:
            if not ln.startswith("info") or " pv " not in ln or " wdl " not in ln:
                continue
            tok = ln.split()
            d = dict(zip(tok[1::1], tok[2::1]))
            mpv = int(d.get("multipv", 1))
            i = tok.index("score")
            cp = mate = None
            if tok[i + 1] == "cp":
                cp = int(tok[i + 2])
            else:
                mate = int(tok[i + 2])
            w = tok.index("wdl")
            wdl = (int(tok[w + 1]), int(tok[w + 2]), int(tok[w + 3]))
            pv = tok[tok.index("pv") + 1]
            last_nodes = int(d.get("nodes", last_nodes))
            # skip bound-only lines (lowerbound/upperbound) when an exact one exists
            if "lowerbound" in tok or "upperbound" in tok:
                if mpv in best:
                    continue
            best[mpv] = PV(pv, wdl, cp, mate, int(d.get("depth", 0)))
        # A node-limited search can stop mid-iteration, leaving stale lines from the previous
        # depth for higher multipv slots. Keep each move once, preferring the deeper line.
        pvs, seen = [], set()
        for k in sorted(best):
            if best[k].move not in seen:
                seen.add(best[k].move)
                pvs.append(best[k])
        return SearchResult(pvs, last_nodes, dt, int(last_nodes / dt) if dt > 0 else 0)

    def close(self):
        try:
            self._send("quit")
            self.p.wait(timeout=5)
        except Exception:
            self.p.kill()


def expected_score(wdl, d: float = 0.5) -> float:
    """wdl in permille or probabilities, from the side to move."""
    w, dr, l = wdl
    s = w + dr + l
    return (w + d * dr) / s
