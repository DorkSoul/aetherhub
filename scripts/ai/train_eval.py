#!/usr/bin/env python3
"""Train the MLP position evaluation and export it to ONNX.

usage: train_eval.py <data.csv> <out.onnx> [hidden=64] [epochs=60]

Rows are `game,turn,seat,won,f0..fN` from `datagen` (features come from engine/crates/mtg-ai/src/features.rs).
Every 5th game is held out. The network is N -> hidden -> hidden/2 -> 1 with ReLU; the exported graph outputs the
win LOGIT (apply a sigmoid outside, like the linear model does). Run with ~/.venvs/aetherhub-ml/bin/python.
"""
import sys
import numpy as np
import torch
import torch.nn as nn

data_path, out_path = sys.argv[1], sys.argv[2]
hidden = int(sys.argv[3]) if len(sys.argv) > 3 else 64
epochs = int(sys.argv[4]) if len(sys.argv) > 4 else 60
torch.manual_seed(0)
np.random.seed(0)

raw = np.loadtxt(data_path, delimiter=",", dtype=np.float32)
game, y, x = raw[:, 0].astype(np.int64), raw[:, 3], raw[:, 4:]
test = game % 5 == 4
xtr, ytr, xte, yte = map(torch.from_numpy, (x[~test], y[~test], x[test], y[test]))
n = x.shape[1]
net = nn.Sequential(nn.Linear(n, hidden), nn.ReLU(), nn.Linear(hidden, hidden // 2), nn.ReLU(), nn.Linear(hidden // 2, 1))
opt = torch.optim.AdamW(net.parameters(), lr=2e-3, weight_decay=1e-3)
loss_fn = nn.BCEWithLogitsLoss()
best, best_state = 9.0, None
for ep in range(epochs):
    net.train()
    perm = torch.randperm(len(xtr))
    for i in range(0, len(perm), 512):
        idx = perm[i:i + 512]
        opt.zero_grad()
        loss_fn(net(xtr[idx]).squeeze(1), ytr[idx]).backward()
        opt.step()
    net.eval()
    with torch.no_grad():
        lo = net(xte).squeeze(1)
        ll = loss_fn(lo, yte).item()
        acc = ((lo > 0).float() == yte).float().mean().item()
    if ll < best:
        best, best_state = ll, {k: v.clone() for k, v in net.state_dict().items()}
    if ep % 10 == 0 or ep == epochs - 1:
        print(f"epoch {ep}: held-out log-loss {ll:.4f} accuracy {acc:.4f}")
net.load_state_dict(best_state)
net.eval()
with torch.no_grad():
    lo = net(xte).squeeze(1)
    print(f"best held-out log-loss {loss_fn(lo, yte).item():.4f} accuracy {((lo > 0).float() == yte).float().mean().item():.4f} ({len(xtr)} train / {len(xte)} test rows)")
torch.onnx.export(net, (torch.zeros(1, n),), out_path, input_names=["features"], output_names=["logit"], dynamic_axes={"features": {0: "batch"}}, dynamo=False)
print("wrote", out_path)
