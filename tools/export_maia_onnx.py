"""Export an official Maia-3 checkpoint to ONNX with move history as an input.

The core needs this file (DECISIONS 0.3): the community 5M export repeats the current board
across the 8 history slots inside the graph, so it can't take real history.

Needs: torch (CPU), onnx, onnxruntime, numpy, and the official package
(`pip install -e <clone of github.com/CSSLab/maia3>`). spikes/.venv has all of them.

    python tools/export_maia_onnx.py --checkpoint maia3-5m.pt --model 5m --out models/maia3-5m.history.fp32.onnx

Inputs:  history f32 [B,64,96] (8 slots x 12 piece channels, oldest first, each slot mirrored
         by its own side to move), elo_self f32 [B], elo_oppo f32 [B]
Outputs: logits_move f32 [B,4352], logits_value f32 [B,3] ([loss, draw, win], side to move)
"""
import argparse
import copy

import numpy as np
import torch

ap = argparse.ArgumentParser()
ap.add_argument("--checkpoint", required=True)
ap.add_argument("--model", default="5m", help="official preset alias: 3m, 5m, 23m, 79m")
ap.add_argument("--out", required=True)
args = ap.parse_args()

from maia3.uci import parse_args, load_model  # noqa: E402  (official package)

cfg = parse_args(["--model", args.model, "--checkpoint-path", args.checkpoint, "--device", "cpu"])
assert not cfg.include_time_info, "time-info checkpoints aren't supported"
model = load_model(cfg)
history = cfg.history


class Wrap(torch.nn.Module):
    def __init__(self, m):
        super().__init__()
        self.m = m

    def forward(self, history, elo_self, elo_oppo):
        lm, lv, _ = self.m(history, elo_self, elo_oppo)
        return lm, lv


# The TorchScript exporter has no aten::rms_norm; trace an equivalent elementwise version.
export_model = copy.deepcopy(model)


def _rms_forward(self, x):
    eps = self.eps if self.eps is not None else torch.finfo(x.dtype).eps
    y = x * torch.rsqrt(x.pow(2).mean(-1, keepdim=True) + eps)
    return y * self.weight if self.weight is not None else y


for mod in export_model.modules():
    if isinstance(mod, torch.nn.RMSNorm):
        mod.forward = _rms_forward.__get__(mod)

dummy = (torch.zeros(2, 64, 12 * history), torch.tensor([1500.0, 1600.0]), torch.tensor([1500.0, 1400.0]))
torch.onnx.export(Wrap(export_model).eval(), dummy, args.out,
                  input_names=["history", "elo_self", "elo_oppo"],
                  output_names=["logits_move", "logits_value"], opset_version=17, dynamo=False,
                  dynamic_axes={n: {0: "batch"} for n in
                                ["history", "elo_self", "elo_oppo", "logits_move", "logits_value"]})

# Check against the unpatched model on random boards.
import onnxruntime as ort  # noqa: E402

rng = np.random.default_rng(0)
x = (rng.random((4, 64, 12 * history)) < 0.05).astype(np.float32)
es = np.array([1100, 1500, 1900, 2300], np.float32)
eo = np.array([1500, 1500, 1200, 2000], np.float32)
with torch.no_grad():
    lm, lv, _ = model(torch.from_numpy(x), torch.from_numpy(es), torch.from_numpy(eo))
sess = ort.InferenceSession(args.out, providers=["CPUExecutionProvider"])
olm, olv = sess.run(None, {"history": x, "elo_self": es, "elo_oppo": eo})
dm, dv = np.abs(olm - lm.numpy()).max(), np.abs(olv - lv.numpy()).max()
print(f"wrote {args.out}; max logit difference vs PyTorch: move {dm:.2e}, value {dv:.2e}")
assert dm < 1e-3 and dv < 1e-3, "export doesn't match the official model"
