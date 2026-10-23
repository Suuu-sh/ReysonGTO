#!/usr/bin/env python3
"""M6 loop for BTN vs BB SRP: preflop solve -> ranges -> postflop hand EV -> realization.

Each round:
  1. solve the 6-max preflop tree with the current realization table;
  2. take BTN's first-in raise range and BB's call range facing it (SB folded);
  3. solve the given flops with those ranges and the size-rule table (--hand-ev-out);
  4. learn SRP realization with tools/m6_realization.py and damp it into the table:
     new = (1 - alpha) * old + alpha * learned. Other pot types move by the same
     ratio as the SRP entries (they are not learned yet).
Rounds resume from what is on disk. Outputs go under --out/round-<k>/.
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path

RANKS = "23456789TJQKA"
SUITS = "cdhs"
POTS = ("limp", "srp", "three_bet", "four_bet")


def class_combos(name):
    a, b = name[0], name[1]
    if len(name) == 2:
        return [a + s + b + t for i, s in enumerate(SUITS) for t in SUITS[i + 1:]]
    if name[2] == "s":
        return [a + s + b + s for s in SUITS]
    return [a + s + b + t for s in SUITS for t in SUITS if s != t]


def node(report, history, actor):
    for n in report["nodes"]:
        if n["history"] == history and n["actor"] == actor:
            return n
    raise SystemExit(f"no preflop node {history} for seat {actor}")


def range_file(n, action_prefix, player, path):
    index = next(i for i, a in enumerate(n["actions"]) if a.startswith(action_prefix))
    combos = {}
    for c in n["classes"]:
        w = c["own_reach_probability"] * c["frequencies"][index]
        for combo in class_combos(c["hand"]):
            combos[combo] = w
    path.write_text(json.dumps({"player": player, "combos": combos}))
    return n["actions"][index]


def run(cmd, log):
    with open(log, "w") as f:
        subprocess.run([str(c) for c in cmd], stdout=f, stderr=subprocess.STDOUT, check=True)


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--rounds", type=int, default=3)
    p.add_argument("--alpha", type=float, default=0.5)
    p.add_argument("--realization", type=Path, required=True, help="starting table")
    p.add_argument("--preflop", type=Path, required=True, help="preflop binary")
    p.add_argument("--config", type=Path, required=True)
    p.add_argument("--equity", type=Path, required=True)
    p.add_argument("--equity-3way", type=Path, required=True)
    p.add_argument("--preflop-iters", type=int, default=100)
    p.add_argument("--cli", type=Path, required=True, help="postflop cli binary")
    p.add_argument("--sizing", type=Path, required=True)
    p.add_argument("--size-rules", type=Path, required=True)
    p.add_argument("--learn", type=Path, required=True, help="tools/m6_realization.py")
    p.add_argument("--flops", required=True)
    p.add_argument("--target", type=float, default=0.5)
    p.add_argument("--threads", type=int, default=10)
    a = p.parse_args()
    table = json.loads(a.realization.read_text())
    for k in range(1, a.rounds + 1):
        d = a.out / f"round-{k}"
        d.mkdir(parents=True, exist_ok=True)
        (d / "realization.json").write_text(json.dumps(table, indent=2))
        report_path = d / "preflop.json"
        if not report_path.exists():
            run([a.preflop, "solve", "--config", a.config, "--equity", a.equity, "--equity-3way", a.equity_3way,
                 "--realization", d / "realization.json", "--max-iters", a.preflop_iters, "--max-seconds", 43200,
                 "--metrics-every", a.preflop_iters, "--target-bb", 0, "--summary-only", "--out", report_path],
                d / "preflop.log")
        report = json.loads(report_path.read_text())
        folds = ["fold"] * 3
        btn = node(report, folds, 3)
        raise_label = range_file(btn, "raise_to", "btn_open", d / "btn_open.json")
        bb = node(report, folds + [raise_label, "fold"], 5)
        range_file(bb, "call", "bb_call", d / "bb_call.json")
        for flop in a.flops.split(","):
            f = d / "flops" / flop
            f.mkdir(parents=True, exist_ok=True)
            if (f / "hand-ev.json").exists():
                continue
            run([a.cli, "solve", "--preset", "production", "--board", flop,
                 "--oop-range", d / "bb_call.json", "--ip-range", d / "btn_open.json",
                 "--pot", 5.5, "--stack", 97.5, "--units-per-bb", 1000, "--no-oop-leads",
                 "--size-rules", a.size_rules, "--storage", "f32", "--threads", a.threads, "--memory-mb", 16384,
                 "--target", a.target, "--max-iters", 10000, "--max-seconds", 7200,
                 "--hand-ev-out", f / "hand-ev.json", "--out", f / "solution.json"], f / "solve.log")
        learned_path = d / "learned.json"
        run([sys.executable, a.learn, d / "flops", "--sizing", a.sizing, "--oop-range", d / "bb_call.json",
             "--ip-range", d / "btn_open.json", "--equal-probability", "--json", learned_path], d / "learn.log")
        learned = json.loads(learned_path.read_text())["realization"]
        side = {"oop": learned["OOP (BB)"], "ip": learned["IP (BTN)"]}
        old_srp = json.loads(json.dumps(table["srp"]))
        for s in ("ip", "oop"):
            for h, old in old_srp[s].items():
                new = (1 - a.alpha) * old + a.alpha * side[s][h]
                for pot in POTS:
                    table[pot][s][h] = round(min(1.5, table[pot][s][h] * new / old), 4)
        summary = report["summary"]["opening_by_position"]
        print(f"round {k}: open " + " ".join(f"{p}={v['raise']*100:.1f}/{v['complete']*100:.1f}" for p, v in summary.items())
              + f"  learned BB={side['oop']['all']:.3f} BTN={side['ip']['all']:.3f}", flush=True)
    (a.out / "realization-final.json").write_text(json.dumps(table, indent=2))


if __name__ == "__main__":
    main()
