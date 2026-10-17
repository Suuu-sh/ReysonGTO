#!/usr/bin/env python3
"""Local progress dashboard for the M4 sizing runs and the overall roadmap.

Reads the runner's output directory (manifest.json, runs/<flop>/stage*-*/) and
roadmap.json on every request; nothing is written. Standard library only.

    python3 tools/dashboard/server.py --data data/m4b --port 8765
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

HERE = Path(__file__).resolve().parent


def read_json(path: Path):
    try:
        with path.open() as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def last_checkpoint(progress: Path):
    """Last checkpoint record of a progress.jsonl, plus a short exploitability trail."""
    try:
        lines = progress.read_text().splitlines()
    except OSError:
        return None, []
    trail, last = [], None
    for line in lines:
        try:
            record = json.loads(line).get("record") or {}
        except ValueError:
            continue
        metrics = record.get("metrics")
        if not metrics:
            continue
        last = record
        trail.append([record.get("elapsed_seconds", 0), metrics.get("exploitability_pct")])
    return last, trail[-60:]


def runner_alive(data: Path) -> bool:
    try:
        out = subprocess.run(["ps", "-eo", "command"], capture_output=True, text=True, timeout=5).stdout
    except (OSError, subprocess.SubprocessError):
        return False
    # The runner may have been started with a relative or an absolute --out.
    suffixes = (f"--out {data}", f"--out data/{data.name}")
    return any("m4_trial.py" in line and any(s in line for s in suffixes) for line in out.splitlines())


def stage_results(board_dir: Path, stage: int):
    out = {}
    for d in sorted(board_dir.glob(f"stage{stage}-*")):
        try:
            size = int(d.name.split("-")[1])
        except (IndexError, ValueError):
            continue
        result = read_json(d / "result.json")
        entry = {"size": size, "status": "pending"}
        if result and result.get("status") == "complete":
            entry.update(
                status="complete",
                ev_bb=result.get("btn_ev_bb"),
                ev_pot_pct=result.get("btn_ev_pot_pct"),
                exploit_pct=result.get("exploitability_pct"),
                seconds=result.get("solve_seconds"),
                rss=(result.get("resource") or {}).get("maximum_resident_set_bytes"),
                finished=(d / "result.json").stat().st_mtime,
            )
        else:
            attempts = sorted(d.glob("attempt-*"), key=lambda p: p.stat().st_mtime)
            if attempts:
                last, trail = last_checkpoint(attempts[-1] / "progress.jsonl")
                entry["status"] = "running"
                entry["started"] = attempts[-1].stat().st_mtime
                if last:
                    entry["live"] = {
                        "iteration": last.get("iteration"),
                        "elapsed": last.get("elapsed_seconds"),
                        "exploit_pct": last["metrics"].get("exploitability_pct"),
                        "trail": trail,
                    }
            elif result:
                entry["status"] = result.get("status", "pending")
        out[size] = entry
    return out


RANKS = "23456789TJQKA"
JOBS: Path | None = None
_board_cache: dict = {}


def hand_class(combo: str) -> str:
    """'AdKd' -> 'AKs', 'Kh9c' -> 'K9o', '7s7d' -> '77'."""
    (r1, s1), (r2, s2) = (combo[0], combo[1]), (combo[2], combo[3])
    if RANKS.index(r1) < RANKS.index(r2):
        r1, r2 = r2, r1
    if r1 == r2:
        return r1 + r2
    return r1 + r2 + ("s" if s1 == s2 else "o")


def action_label(action) -> str:
    if isinstance(action, str):
        return action
    (kind, amount), = action.items()
    return f"{kind} {amount}"


def board_detail(data: Path, flop: str):
    """BTN's first flop decision (after the forced check) for each finished top-two size."""
    out = []
    for d in sorted((data / "runs" / flop).glob("stage2-*")):
        result = read_json(d / "result.json")
        if not result or result.get("status") != "complete":
            continue
        path = Path(result.get("solution_path") or "")
        try:
            mtime = path.stat().st_mtime
        except OSError:
            continue
        key = (str(path), mtime)
        if key not in _board_cache:
            solution = read_json(path) or {}
            node = solution.get("after_check_strategy") or {}
            actions = [action_label(a) for a in node.get("actions", [])]
            classes: dict = {}
            total = [0.0] * len(actions)
            reach_sum = 0.0
            for combo in node.get("combos", []):
                w = combo.get("reach_weight") or 0.0
                c = classes.setdefault(hand_class(combo["combo"]), {"reach": 0.0, "mix": [0.0] * len(actions)})
                c["reach"] += w
                reach_sum += w
                for i, p in enumerate(combo["probabilities"]):
                    c["mix"][i] += w * p
                    total[i] += w * p
            for c in classes.values():
                c["mix"] = [m / c["reach"] if c["reach"] else 0.0 for m in c["mix"]]
            # Keep the cache to the boards someone actually opened recently.
            if len(_board_cache) > 64:
                _board_cache.clear()
            _board_cache[key] = {
                "size": result["size"],
                "ev_bb": result.get("btn_ev_bb"),
                "exploit_pct": result.get("exploitability_pct"),
                "actions": actions,
                "overall": [t / reach_sum if reach_sum else 0.0 for t in total],
                "classes": classes,
            }
        out.append(_board_cache[key])
    return sorted(out, key=lambda r: -(r["ev_bb"] or 0))


def time_l(stderr: Path):
    """Wall seconds and peak RSS from a macOS `/usr/bin/time -l` trailer."""
    wall = rss = None
    try:
        for line in stderr.read_text(errors="replace").splitlines():
            parts = line.split()
            if len(parts) >= 2 and parts[1] == "real":
                wall = float(parts[0])
            elif line.strip().endswith("maximum resident set size"):
                rss = int(parts[0])
    except (OSError, ValueError):
        pass
    return wall, rss


def process_running(needle: str) -> bool:
    try:
        out = subprocess.run(["ps", "-eo", "args="], capture_output=True, text=True, timeout=5).stdout
    except (OSError, subprocess.SubprocessError):
        return False
    return any(needle in line for line in out.splitlines())


def tail(path: Path, n: int = 6):
    try:
        lines = [l for l in path.read_text(errors="replace").splitlines() if l.strip()]
    except OSError:
        return []
    return lines[-n:]


F32_BOARDS = ("3d3c2c", "7c4d2h", "Qc6d5c")
F32_MODES = ("f64", "f32", "wide")


def f32_job(root: Path):
    log = tail(root / "log.txt", 20)
    runs = []
    for board in F32_BOARDS:
        for mode in F32_MODES:
            d = root / f"{board}-{mode}"
            run = {"board": board, "mode": mode, "state": "pending"}
            ended = any(l.startswith(f"{board} {mode} exit=") for l in log)
            if d.exists():
                last, trail = last_checkpoint(d / "progress.jsonl")
                run["state"] = "done" if ended else "running"
                if last:
                    run.update(iteration=last.get("iteration"), elapsed=last.get("elapsed_seconds"),
                               exploit_pct=last["metrics"].get("exploitability_pct"), trail=trail)
                if ended:
                    wall, rss = time_l(d / "stderr.log")
                    solution = read_json(d / "solution.json") or {}
                    ev = solution.get("root_ev_chips") or [None, None]
                    units = ((solution.get("game_spec") or {}).get("units_per_bb")) or 1000
                    run.update(wall=wall, rss=rss, iterations=solution.get("iterations"),
                               exploit_pct=(solution.get("metrics") or {}).get("exploitability_pct", run.get("exploit_pct")),
                               btn_ev_bb=ev[1] / units if ev[1] is not None else None)
                    if not solution:
                        run["state"] = "failed"
            runs.append(run)
    return {"runs": runs, "log": log[-6:]}


def p4_job(root: Path):
    log = tail(root / "log.txt", 20)
    def step(name, marker):
        if any(l.startswith(marker) for l in log):
            ok = any(l.startswith(marker) and "exit=0" in l for l in log)
            return {"name": name, "state": "done" if ok else "failed"}
        return {"name": name, "state": "pending"}
    steps = [
        {"name": "f32 の比較が終わるのを待つ", "state": "done" if log else "waiting"},
        step("3 人のエクイティ表を作る", "equity exit="),
        step("6 人の木を 200 反復解く", "solve exit="),
    ]
    if log and steps[1]["state"] == "pending":
        steps[1]["state"] = "running"
    elif steps[1]["state"] == "done" and steps[2]["state"] == "pending":
        steps[2]["state"] = "running" if process_running("preflop solve") else "stopped"
    return {"steps": steps, "log": log[-6:], "equity_tail": tail(root / "equity.err", 3),
            "solve_tail": tail(root / "solve.err", 4)}


def running_attempt(root: Path):
    """Newest attempt directory without a result: the solve in progress, if any."""
    attempts = [d for d in root.glob("**/attempt-*") if d.is_dir() and not (d / "result.json").exists()]
    if not attempts:
        return None
    d = max(attempts, key=lambda p: p.stat().st_mtime)
    last, trail = last_checkpoint(d / "progress.jsonl")
    rel = d.relative_to(root).parts
    info = {"path": "/".join(rel[:-1]), "started": d.stat().st_mtime}
    if last:
        info.update(iteration=last.get("iteration"), elapsed=last.get("elapsed_seconds"),
                    exploit_pct=last["metrics"].get("exploitability_pct"), trail=trail)
    return info


def turn_job(root: Path):
    data = root / "data"
    log = tail(root / "log.txt", 10)
    flops, spots, expected = [], [], 0
    for cards in sorted((data / "cards").glob("*/result.json")):
        record = read_json(cards) or {}
        groups = ((record.get("cards") or {}).get("groups")) or []
        n_cards = sum(len(g.get("cards", [])) for g in groups)
        expected += n_cards * 2 * 2  # lines (XX, BC) x bettors (OOP lead, IP after check)
        flop = cards.parent.name
        flop_result = read_json(data / "flops" / flop / "result.json") or {}
        flops.append({"flop": flop, "status": flop_result.get("status", "running"),
                      "kinds": {g["kind"]: g.get("cards", []) for g in groups}})
    for summary in sorted((data / "spots").glob("*/summary.json")):
        s = read_json(summary) or {}
        fine = {r.get("size"): r.get("bettor_ev_bb") for r in s.get("fine", [])}
        spots.append({k: s.get(k) for k in ("flop", "line", "card", "kind", "bettor", "complete",
                                             "observed_winner", "gap_bb", "indistinguishable_by_proxy")}
                     | {"fine": fine,
                        "seconds": sum(((r.get("resource") or {}).get("wall_seconds") or 0)
                                       for r in s.get("coarse", []) + s.get("fine", []))})
    return {"log": log[-4:], "flops": flops, "spots": spots, "expected": expected,
            "running": running_attempt(data) if data.exists() else None,
            "finished": any(l.startswith("done") for l in log)}


def codex_jobs():
    """`codex exec` processes and the worktree each is working in."""
    try:
        out = subprocess.run(["ps", "-eo", "etime=,args="], capture_output=True, text=True, timeout=5).stdout
    except (OSError, subprocess.SubprocessError):
        return []
    jobs = []
    for line in out.splitlines():
        parts = line.split()
        if len(parts) > 3 and parts[1].endswith("codex") and "exec" in parts[2:4] and "-C" in parts:
            jobs.append({"elapsed": parts[0], "dir": Path(parts[parts.index("-C") + 1]).name})
    return jobs


def collect_jobs(jobs: Path | None):
    if not jobs:
        return None
    return {"f32cmp": f32_job(jobs / "f32cmp"), "p4": p4_job(jobs / "p4"),
            "turn": turn_job(jobs / "turnpilot"), "codex": codex_jobs()}


def collect(data: Path):
    manifest = read_json(data / "manifest.json") or {}
    selection = manifest.get("selection") or {}
    training = selection.get("training") or []
    # The runner solves the training boards first, then the held-out validation set.
    items = [(item, "training") for item in training] + [
        (item, "validation") for item in selection.get("validation") or []]
    boards, running = [], None
    for item, role in items:
        flop = item["flop"]
        bdir = data / "runs" / flop
        s1 = stage_results(bdir, 1) if bdir.exists() else {}
        s2 = stage_results(bdir, 2) if bdir.exists() else {}
        done2 = [r for r in s2.values() if r["status"] == "complete"]
        state = "pending"
        if s1 or s2:
            state = "running"
        if len(done2) >= 2:
            state = "done"
        winner = margin = None
        if state == "done":
            ranked = sorted(done2, key=lambda r: r["ev_bb"], reverse=True)
            winner = ranked[0]["size"]
            margin = ranked[0]["ev_bb"] - ranked[1]["ev_bb"]
        finished = max((r.get("finished", 0) for r in done2), default=None) if state == "done" else None
        board = {
            "flop": flop,
            "role": role,
            "features": item.get("features", {}),
            "weight": item.get("weight"),
            "state": state,
            "stage1": list(s1.values()),
            "stage2": list(s2.values()),
            "winner": winner,
            "margin_bb": margin,
            "finished": finished,
        }
        for stage, results in ((1, s1), (2, s2)):
            for r in results.values():
                if r["status"] == "running":
                    running = {"flop": flop, "stage": stage, **r}
        boards.append(board)

    done = [b for b in boards if b["state"] == "done"]
    finished_times = sorted(b["finished"] for b in done if b["finished"])
    created = manifest.get("created_utc")
    rate = None
    # Recent pace: the last 10 completions, so a slow start or pause doesn't skew it.
    if len(finished_times) >= 2:
        window = finished_times[-11:]
        rate = (window[-1] - window[0]) / (len(window) - 1)
    remaining = len(boards) - len(done)
    return {
        "now": time.time(),
        "data": str(data),
        "created_utc": created,
        "deadline": manifest.get("budget_deadline_unix"),
        "alive": runner_alive(data),
        "total": len(boards),
        "done": len(done),
        "training_total": len(training),
        "training_done": sum(b["state"] == "done" for b in boards if b["role"] == "training"),
        "validation_total": len(items) - len(training),
        "validation_done": sum(b["state"] == "done" for b in boards if b["role"] == "validation"),
        "seconds_per_board": rate,
        "eta": (time.time() + rate * remaining) if rate and remaining else None,
        "first_finished": finished_times[0] if finished_times else None,
        "running": running,
        "boards": boards,
        "roadmap": read_json(HERE / "roadmap.json"),
        "jobs": collect_jobs(JOBS),
    }


class Handler(BaseHTTPRequestHandler):
    data: Path

    def do_GET(self):
        if self.path.startswith("/api/status"):
            body = json.dumps(collect(self.data)).encode()
            ctype = "application/json"
        elif self.path.startswith("/api/board?flop="):
            flop = self.path.split("=", 1)[1]
            if not (len(flop) == 6 and flop.isalnum()):
                self.send_error(400)
                return
            body = json.dumps(board_detail(self.data, flop)).encode()
            ctype = "application/json"
        elif self.path in ("/", "/index.html"):
            body = (HERE / "index.html").read_bytes()
            ctype = "text/html; charset=utf-8"
        else:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--data", default=os.environ.get("M4_DATA", "data/m4b"))
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--jobs", help="directory holding the f32cmp/ and p4/ background job outputs")
    args = parser.parse_args()
    Handler.data = Path(args.data).resolve()
    global JOBS
    JOBS = Path(args.jobs).resolve() if args.jobs else None
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    print(f"http://127.0.0.1:{args.port}/  (data: {Handler.data})", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
