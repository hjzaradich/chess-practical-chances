"""Spike 3: Maia's draw estimate D_R on the Purpose-table sanity positions at 1100/1500/1900,
with real move history (where the position has one) and with the board replicated.
Also prints Maia's top moves in the trap positions and Stockfish's WDL for comparison.

Run: .venv/Scripts/python s3_draw.py
"""
from common import MODELS, MaiaOnnx, Stockfish, boards_from_moves
from positions import SANITY

RATINGS = [1100, 1500, 1900]
maia = MaiaOnnx(MODELS / "maia3-5m.history.fp32.onnx", history_input=True)
sf = Stockfish(threads=1)

print("D_R = Maia value-head draw probability, side to move's view; same rating both sides.")
print("hist = real game history; rep = current board only (FEN input).  SF = Stockfish 1M nodes W/D/L.\n")
print(f"{'position':30s}" + "".join(f"{'D'+str(r):>8s}" for r in RATINGS) +
      "".join(f"{'Wmaia'+str(r):>11s}" for r in RATINGS) + f"{'D rep1500':>10s}   SF W/D/L")
rows = {}
for name, fen, moves in SANITY:
    boards = boards_from_moves(fen, moves)
    ds, ws = [], []
    for r in RATINGS:
        o = maia.evaluate([boards], r, r)[0]
        ds.append(o.wdl[1]); ws.append(o.wdl[0])
    rep = maia.evaluate([[boards[-1]]], 1500, 1500)[0].wdl[1]
    s = sf.search(boards[-1].fen(), 1_000_000).pvs[0].wdl
    rows[name] = ds
    print(f"{name:30s}" + "".join(f"{d:8.3f}" for d in ds) + "".join(f"{w:11.3f}" for w in ws)
          + f"{rep:10.3f}   {s[0]}/{s[1]}/{s[2]}")

print("\nchecks (1500):")
print("  rook_vs_rook D > najdorf D:", rows["rook_vs_rook"][1] > rows["najdorf_english_attack"][1])
print("  blocked_pawns D > najdorf D:", rows["blocked_pawns"][1] > rows["najdorf_english_attack"][1])
print("  berlin D > najdorf D:", rows["berlin_endgame"][1] > rows["najdorf_english_attack"][1])

print("\nMaia's likely moves in the trap positions (P by rating):")
for name, fen, moves in SANITY:
    if not name.startswith("trap"):
        continue
    boards = boards_from_moves(fen, moves)
    b = boards[-1]
    outs = [maia.evaluate([boards], r, r)[0] for r in RATINGS]
    top = outs[1].moves[:6]
    print(f"  {name}:")
    for m in top:
        ps = []
        for o in outs:
            i = o.moves.index(m)
            ps.append(o.probs[i])
        print(f"    {b.san(m):6s} " + "  ".join(f"{r}:{p:.2f}" for r, p in zip(RATINGS, ps)))
sf.close()
