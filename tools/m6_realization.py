#!/usr/bin/env python3
"""Learn flop realization factors (M6, v2) from postflop solves.

Reads `--hand-ev-out` files (one per flop) and reports, per player and hand
category of the preflop realization table, the ratio

    R = sum w * (ev + pot/2)  /  sum w * equity * (pot - rake(pot))

where w is the combo's reach weight within its flop times the flop's population
weight: canonical multiplicity x the compatible joint mass of the two original
(pre-board) ranges on that board x the sampling design weight (1 / inclusion
probability). EV, pot and rake are converted to BB before summing.
The denominator matches the preflop model's flop terminal value
(equity * available * R, available = pot minus rake on the flop pot), so R
can replace that table's entries directly.

    python3 tools/m6_realization.py data/jobs/m6pilot --sizing target/release/sizing \\
        --oop-range data/ranges/bb_call.json --ip-range data/ranges/btn_open.json --equal-probability
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


def load_range(path):
    data = json.loads(Path(path).read_text())
    combos = data["combos"]
    return {k: float(v) for k, v in combos.items()} if isinstance(combos, dict) else \
        {c["combo"]: float(c["weight"]) for c in combos}


def joint_mass(oop, ip, board):
    """Sum of w_oop * w_ip over combo pairs that avoid the board and each other."""
    dead = {board[i:i + 2] for i in range(0, len(board), 2)}
    live = lambda r: {k: w for k, w in r.items() if k[:2] not in dead and k[2:] not in dead and w > 0}
    oop, ip = live(oop), live(ip)
    total_ip = sum(ip.values())
    by_card = defaultdict(float)
    for k, w in ip.items():
        by_card[k[:2]] += w
        by_card[k[2:]] += w
    mass = 0.0
    for k, w in oop.items():
        # Inclusion-exclusion: the identical combo was subtracted once per card.
        blocked = by_card[k[:2]] + by_card[k[2:]] - ip.get(k, 0.0) - ip.get(k[2:] + k[:2], 0.0)
        mass += w * (total_ip - blocked)
    return mass


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
    parser.add_argument("--oop-range", required=True, help="original OOP range the solves used")
    parser.add_argument("--ip-range", required=True, help="original IP range the solves used")
    design = parser.add_mutually_exclusive_group(required=True)
    design.add_argument("--design-weights", type=Path,
                        help="JSON {flop: 1 / inclusion probability} of the flop sample")
    design.add_argument("--equal-probability", action="store_true",
                        help="the flops are a census or an equal-probability sample")
    args = parser.parse_args()
    weights = flop_weights(args.sizing)
    oop_range, ip_range = load_range(args.oop_range), load_range(args.ip_range)
    design = json.loads(args.design_weights.read_text()) if args.design_weights else None
    num = defaultdict(float)
    den = defaultdict(float)
    flops = []
    paths = sorted(args.root.glob("*/hand-ev.json"))
    if not paths:
        raise SystemExit(f"no */hand-ev.json under {args.root}")
    for path in paths:
        flop = path.parent.name
        report = json.loads(path.read_text())
        units = report["units_per_bb"]
        pot = report["pot_chips"] / units
        available = pot - min(pot * args.rake, args.rake_cap_bb)
        if design is not None and flop not in design:
            raise SystemExit(f"{flop} has no design weight")
        fw = weights[flop] * joint_mass(oop_range, ip_range, flop) * (design[flop] if design else 1.0)
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
                    num[key] += w * (c["ev_chips"] / units + pot / 2)
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
