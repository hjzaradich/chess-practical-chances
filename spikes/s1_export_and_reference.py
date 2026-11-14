"""Spike 1a: export Maia3-5M to ONNX with real history as an input, and check it (and the
community export) against the official PyTorch package. Writes reference outputs that the
Rust spike must reproduce.

Run: .venv/Scripts/python s1_export_and_reference.py
"""
import json
import sys

import numpy as np
import torch

from common import (MODELS, HISTORY, MaiaOnnx, boards_from_moves, history_tokens,
                    legal_index, softmax, tokenize)
from positions import PARITY

sys.argv = sys.argv[:1]
from maia3.uci import parse_args, load_model  # official package
from maia3.dataset import tokenize_board, get_historical_tokens

CKPT = MODELS / "maia3-5m.pt"
OURS = MODELS / "maia3-5m.history.fp32.onnx"
COMMUNITY = MODELS / "maia3-5m.fp32.onnx"
RATINGS = [1100, 1500, 1900]

cfg = parse_args(["--model", "5m", "--checkpoint-path", str(CKPT), "--device", "cpu"])
model = load_model(cfg)
print("official cfg: history", cfg.history, "dim_vit", cfg.dim_vit, "heads", cfg.num_heads,
      "time_info", cfg.include_time_info)


class Wrap(torch.nn.Module):
    def __init__(self, m):
        super().__init__()
        self.m = m

    def forward(self, history, elo_self, elo_oppo):
        lm, lv, _ = self.m(history, elo_self, elo_oppo)
        return lm, lv


def official_tokens(boards):
    """Encode with the official package's own functions (not our numpy copy)."""
    from collections import deque
    dq = deque(maxlen=cfg.history)
    for b in boards:
        dq.append(tokenize_board(b))
    t = get_historical_tokens(dq, cfg, 0.0, 0.0, 0.0, 0.0)  # (64, 97): 96 + clk_ponder
    return t


# ---- export
# The TorchScript exporter has no aten::rms_norm; trace an equivalent elementwise RMSNorm.
# Parity below is checked against the *unpatched* model, so this can't hide a mismatch.
import copy
export_model = copy.deepcopy(model)


def _rms_forward(self, x):
    eps = self.eps if self.eps is not None else torch.finfo(x.dtype).eps
    y = x * torch.rsqrt(x.pow(2).mean(-1, keepdim=True) + eps)
    return y * self.weight if self.weight is not None else y


for mod in export_model.modules():
    if isinstance(mod, torch.nn.RMSNorm):
        mod.forward = _rms_forward.__get__(mod)
w = Wrap(export_model).eval()
dummy = (torch.zeros(2, 64, 12 * HISTORY), torch.tensor([1500.0, 1600.0]), torch.tensor([1500.0, 1400.0]))
torch.onnx.export(w, dummy, str(OURS), input_names=["history", "elo_self", "elo_oppo"],
                  output_names=["logits_move", "logits_value"], opset_version=17, dynamo=False,
                  dynamic_axes={"history": {0: "batch"}, "elo_self": {0: "batch"},
                                "elo_oppo": {0: "batch"}, "logits_move": {0: "batch"},
                                "logits_value": {0: "batch"}})
print("exported", OURS, OURS.stat().st_size // 1024, "KB")

ours = MaiaOnnx(OURS, history_input=True)
comm = MaiaOnnx(COMMUNITY, history_input=False)

max_enc = 0.0
max_ours_p = max_ours_v = 0.0
max_comm_p = max_comm_v = 0.0
hist_effect = []
reference = []

with torch.no_grad():
    for name, fen, moves in PARITY:
        boards = boards_from_moves(fen, moves)
        board = boards[-1]
        off = official_tokens(boards)[:, :96].numpy()
        mine = history_tokens(boards)
        max_enc = max(max_enc, float(np.abs(off - mine).max()))
        legal, idx = legal_index(board)
        for r in RATINGS:
            # torch, real history (as the official UCI engine does with --use-uci-history)
            lm, lv, _ = model(torch.from_numpy(off)[None], torch.tensor([r]), torch.tensor([r]))
            p_t = softmax(lm[0].numpy()[idx].astype(np.float64))
            v_t = softmax(lv[0].numpy().astype(np.float64))  # loss, draw, win
            # torch, replicated current board (official UCI default)
            rep = np.tile(tokenize(board), (1, HISTORY))
            lm2, lv2, _ = model(torch.from_numpy(rep)[None], torch.tensor([r]), torch.tensor([r]))
            p_rep = softmax(lm2[0].numpy()[idx].astype(np.float64))
            v_rep = softmax(lv2[0].numpy().astype(np.float64))
            # ours (onnx, history)
            lm3, lv3 = ours.raw(mine[None], np.array([r], np.float32), np.array([r], np.float32))
            p_o = softmax(lm3[0][idx].astype(np.float64)); v_o = softmax(lv3[0].astype(np.float64))
            max_ours_p = max(max_ours_p, float(np.abs(p_o - p_t).max()))
            max_ours_v = max(max_ours_v, float(np.abs(v_o - v_t).max()))
            # community (onnx, replicated) vs torch replicated
            lm4, lv4 = comm.raw(tokenize(board)[None], np.array([r], np.float32), np.array([r], np.float32))
            p_c = softmax(lm4[0][idx].astype(np.float64)); v_c = softmax(lv4[0].astype(np.float64))
            max_comm_p = max(max_comm_p, float(np.abs(p_c - p_rep).max()))
            max_comm_v = max(max_comm_v, float(np.abs(v_c - v_rep).max()))
            if moves:
                hist_effect.append((name, r, float(np.abs(p_t - p_rep).max()),
                                    float(abs(v_t[1] - v_rep[1]))))
            reference.append({
                "name": name, "start_fen": fen, "moves": moves, "elo": r,
                "fen": board.fen(),
                "probs": {m.uci(): float(p) for m, p in zip(legal, p_t)},
                "wdl": [float(v_t[2]), float(v_t[1]), float(v_t[0])],  # win, draw, loss
            })

print(f"encoding: our numpy encoder vs official tokens, max abs diff = {max_enc}")
print(f"our history ONNX vs torch (real history): move-prob {max_ours_p:.2e}, value {max_ours_v:.2e}")
print(f"community ONNX vs torch (replicated board): move-prob {max_comm_p:.2e}, value {max_comm_v:.2e}")
print("effect of real history vs replicated board (max move-prob diff, draw diff):")
for name, r, dp, dd in hist_effect:
    print(f"  {name:28s} {r}: dp={dp:.3f} dD={dd:.3f}")

(MODELS.parent.parent / "s1_reference.json").write_text(json.dumps(reference, indent=1))
print("wrote s1_reference.json with", len(reference), "cases")
