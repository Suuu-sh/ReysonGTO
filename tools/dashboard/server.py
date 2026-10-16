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
    }


class Handler(BaseHTTPRequestHandler):
    data: Path

    def do_GET(self):
        if self.path.startswith("/api/status"):
            body = json.dumps(collect(self.data)).encode()
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
    args = parser.parse_args()
    Handler.data = Path(args.data).resolve()
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    print(f"http://127.0.0.1:{args.port}/  (data: {Handler.data})", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
