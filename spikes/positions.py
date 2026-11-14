"""Test positions for the Phase 0 spikes. Each is (name, start_fen or None, uci_moves).
Positions given by move list carry real history for Maia; FEN-only ones get padded history."""
import chess


def san_to_uci(start_fen, sans):
    b = chess.Board(start_fen) if start_fen else chess.Board()
    out = []
    for s in sans.split():
        if s[0].isdigit():
            s = s.split(".")[-1]
            if not s:
                continue
        m = b.parse_san(s)
        out.append(m.uci())
        b.push(m)
    return out


NAJDORF = san_to_uci(None, "1.e4 c5 2.Nf3 d6 3.d4 cxd4 4.Nxd4 Nf6 5.Nc3 a6 6.Be3 e5 7.Nb3 Be6 "
                           "8.f3 Be7 9.Qd2 O-O 10.O-O-O Nbd7 11.g4 b5 12.g5 b4")
BERLIN = san_to_uci(None, "1.e4 e5 2.Nf3 Nc6 3.Bb5 Nf6 4.O-O Nxe4 5.d4 Nd6 6.Bxc6 dxc6 "
                          "7.dxe5 Nf5 8.Qxd8+ Kxd8")
# Blackburne-Shilling: after 3...Nd4 White's natural 4.Nxe5? walks into ...Qg5.
SHILLING = san_to_uci(None, "1.e4 e5 2.Nf3 Nc6 3.Bc4 Nd4")
# Englund: after 3...Qe7 White's natural 4.Bf4? Qb4+ 5.Bd2 Qxb2 6.Bc3? Bb4! wins.
ENGLUND = san_to_uci(None, "1.d4 e5 2.dxe5 Nc6 3.Nf3 Qe7")
RUY = san_to_uci(None, "1.e4 e5 2.Nf3 Nc6 3.Bb5 a6 4.Ba4 Nf6 5.O-O Be7 6.Re1 b5 7.Bb3 d6 8.c3 O-O 9.h3")
EP = san_to_uci(None, "1.e4 Nf6 2.e5 d5")  # en passant exf6/exd6 available

# (name, start_fen, moves, attacker-or-trapper note)
SANITY = [
    ("rook_vs_rook", "8/3k4/8/8/8/4K3/r7/5R2 w - - 0 1", []),
    ("blocked_pawns", "4k3/8/1p1p1p1p/pPpPpPpP/P1P1P1P1/8/8/4K3 w - - 0 1", []),
    ("najdorf_english_attack", None, NAJDORF),
    ("berlin_endgame", None, BERLIN),
    ("rb_vs_r_white_to_move", "8/7r/3k4/8/4K3/3B4/8/5R2 w - - 0 1", []),
    ("rb_vs_r_black_to_move", "8/7r/3k4/8/4K3/3B4/8/5R2 b - - 0 1", []),
    ("trap_shilling_white_to_move", None, SHILLING),
    ("trap_englund_white_to_move", None, ENGLUND),
]

# For the Rust/ONNX comparison: varied encodings (black to move, promotions, castling, ep).
PARITY = SANITY + [
    ("startpos", None, []),
    ("ruy_closed", None, RUY),
    ("en_passant", None, EP),
    ("promotion_white", "8/1P4k1/8/8/8/8/6K1/8 w - - 0 1", []),
    ("promotion_black", "8/6K1/8/8/8/8/1p4k1/8 b - - 0 1", []),
    ("underpromo_capture", "1r5k/P7/8/8/8/8/8/K7 w - - 0 1", []),
]
