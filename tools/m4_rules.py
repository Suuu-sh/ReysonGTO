#!/usr/bin/env python3
"""Evaluate flop sizing rules on an M4 run directory (training vs validation boards).

Per board, a size's EV is the stage-2 EV when it was in the top two; otherwise its
stage-1 EV shifted by the mean stage-2 minus stage-1 offset of the top two. Loss is
the best size's EV minus the chosen size's EV, as % of the 5.5BB pot, weighted by
the flop's combinatorial weight.

    python3 tools/m4_rules.py data/m4b
"""
import glob
import itertools
import json
import statistics
import sys

SIZES = (33, 66, 100, 125)
POT_BB = 5.5
FEATURES = ("suits", "pairing", "height", "connectivity")


def load(root):
    selection = json.load(open(f"{root}/manifest.json"))["selection"]
    boards = []
    for role in ("training", "validation"):
        for item in selection.get(role, []):
            stages = {1: {}, 2: {}}
            for stage in stages:
                for path in glob.glob(f"{root}/runs/{item['flop']}/stage{stage}-*/result.json"):
                    result = json.load(open(path))
                    if result.get("status") == "complete":
                        stages[stage][result["size"]] = result["btn_ev_bb"]
            if len(stages[2]) < 2 or len(stages[1]) < len(SIZES):
                continue
            offset = statistics.mean(stages[2][z] - stages[1][z] for z in stages[2])
            ev = {z: stages[2].get(z, stages[1][z] + offset) for z in SIZES}
            best = max(ev.values())
            boards.append({"flop": item["flop"], "role": role, "weight": item["weight"],
                           "features": item["features"], "loss": {z: best - ev[z] for z in SIZES}})
    return boards


def mean_loss(boards, pick):
    total = sum(b["weight"] for b in boards)
    return sum(b["weight"] * b["loss"][pick(b["features"])] for b in boards) / total


def max_loss(boards, pick):
    return max(b["loss"][pick(b["features"])] for b in boards)


def pct(bb):
    return f"{bb / POT_BB * 100:.3f}%"


def main(root):
    boards = load(root)
    train = [b for b in boards if b["role"] == "training"]
    valid = [b for b in boards if b["role"] == "validation"]
    print(f"boards: training {len(train)}, validation {len(valid)}")
    rules = {"33% everywhere": lambda f: 33,
             "66% on connected unpaired 8-high or lower, else 33%":
                 lambda f: 66 if f["connectivity"] == "Connected" and f["pairing"] == "Unpaired"
                 and f["height"] in ("Low", "TenEight") else 33}
    # Fitted tables over every one- and two-feature split, to show what extra rows buy.
    for n in (1, 2):
        for keys in itertools.combinations(FEATURES, n):
            groups = {}
            for b in train:
                groups.setdefault(tuple(b["features"][k] for k in keys), []).append(b)
            table = {g: min(SIZES, key=lambda z: mean_loss(bs, lambda f, z=z: z)) for g, bs in groups.items()}
            rules[f"fitted by {'+'.join(keys)} ({len(table)} rows)"] = (
                lambda f, keys=keys, table=table: table.get(tuple(f[k] for k in keys), 33))
    print(f"{'rule':58s} {'train mean':>10s} {'train max':>10s} {'valid mean':>10s} {'valid max':>10s}")
    for name, pick in rules.items():
        print(f"{name:58s} {pct(mean_loss(train, pick)):>10s} {pct(max_loss(train, pick)):>10s} "
              f"{pct(mean_loss(valid, pick)):>10s} {pct(max_loss(valid, pick)):>10s}")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "data/m4b")
