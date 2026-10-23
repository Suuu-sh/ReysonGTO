#!/usr/bin/env python3
"""Learn flop realization factors (M6, v2) from postflop solves.

Reads `--hand-ev-out` files (one per flop) and reports, per player and hand
category of the preflop realization table, the ratio

    R = sum w * (ev + pot/2)  /  sum w * equity * (pot - rake(pot))

where w is the combo's reach weight times the flop's combinatorial weight.
The denominator matches the preflop model's flop terminal value
(equity * available * R, available = pot minus rake on the flop pot), so R
can replace that table's entries directly.

    python3 tools/m6_realization.py data/jobs/m6pilot --sizing target/release/sizing
"""
import argparse
import json
import subprocess
from collections import defaultdict
from pathlib import Path

RANKS = "23456789TJQKA"
CATEGORIES = ("pair", "suited_connector", "suited", "offsuit_broadway", "offsuit")


def category(hand_class):
    """Mirror of preflop::realization::RealizationConfig::get."""
    if len(hand_class) == 2:
        return "pair"
    a, b = RANKS.index(hand_class[0]), RANKS.index(hand_class[1])
    if hand_class[2] == "s":
        return "suited_connector" if abs(a - b) == 1 else "suited"
    if hand_class[0] in "TJQKA" and hand_class[1] in "TJQKA":
        return "offsuit_broadway"
    return "offsuit"


def flop_weights(sizing):
    rows = json.loads(subprocess.run([sizing, "flops"], capture_output=True, text=True, check=True).stdout)
    rows = rows.get("flops", rows) if isinstance(rows, dict) else rows
    return {r["flop"]: r["weight"] for r in rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("root", type=Path, help="directory with <flop>/hand-ev.json")
    parser.add_argument("--sizing", default="target/release/sizing")
    parser.add_argument("--rake", type=float, default=0.05)
    parser.add_argument("--rake-cap-bb", type=float, default=3.0)
    parser.add_argument("--json", type=Path, help="write the learned table here")
    args = parser.parse_args()
    weights = flop_weights(args.sizing)
    num = defaultdict(float)
    den = defaultdict(float)
    flops = []
    for path in sorted(args.root.glob("*/hand-ev.json")):
        flop = path.parent.name
        report = json.loads(path.read_text())
        pot = report["pot_chips"]
        available = pot - min(pot * args.rake, args.rake_cap_bb * report["units_per_bb"])
        fw = weights[flop]
        flops.append(flop)
        for player in report["players"]:
            p = player["player"]
            total = sum(c["reach_weight"] or 0 for c in player["combos"])
            for c in player["combos"]:
                w = c["reach_weight"]
                if not w or c["ev_chips"] is None:
                    continue
                w = fw * w / total
                for key in ((p, category(c["hand_class"])), (p, "all"), (p, c["hand_class"])):
                    num[key] += w * (c["ev_chips"] + pot / 2)
                    den[key] += w * c["equity"] * available
    table = {}
    print(f"flops: {len(flops)} ({', '.join(flops)})")
    for p, name in ((0, "OOP (BB)"), (1, "IP (BTN)")):
        row = {k: num[(p, k)] / den[(p, k)] for k in CATEGORIES + ("all",) if den[(p, k)]}
        table[name] = row
        print(name, "  ".join(f"{k}={v:.3f}" for k, v in row.items()))
    if args.json:
        args.json.write_text(json.dumps({"flops": flops, "realization": table}, indent=2))


if __name__ == "__main__":
    main()
