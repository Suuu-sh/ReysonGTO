#!/usr/bin/env python3
"""Measure one foreground command; no polling, concurrent runs, or dependencies.

On this sandbox /usr/bin/time -l cannot read kern.clockrate, so getrusage is
used for peak RSS. Command output is inherited (redirect it in the shell).
The optional timeout kills and reaps the direct child; descendant process
termination is not promised.
"""
import argparse
import json
import math
import platform
import resource
import subprocess
import sys
import time
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--metrics", type=Path, required=True)
    parser.add_argument("--timeout-seconds", type=float, default=None,
                        help="Hard wall timeout for the direct child (positive and finite)")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.timeout_seconds is not None and (
        not math.isfinite(args.timeout_seconds) or args.timeout_seconds <= 0
    ):
        parser.error("--timeout-seconds must be positive and finite")
    command = args.command
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        parser.error("provide a command after --")
    start = time.monotonic()
    timed_out = False
    try:
        result = subprocess.run(command, check=False, timeout=args.timeout_seconds)
        exit_code = result.returncode
    except subprocess.TimeoutExpired:
        # subprocess.run kills and waits for its direct child before re-raising.
        timed_out = True
        exit_code = 124
        print("Command exceeded --timeout-seconds; direct child killed and reaped",
              file=sys.stderr)
    elapsed = time.monotonic() - start
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    # Darwin reports bytes; Linux reports KiB. Refuse ambiguous platforms.
    system = platform.system()
    if system not in ("Darwin", "Linux"):
        raise RuntimeError(f"unknown ru_maxrss units on {system}")
    metrics = {
        "command": command,
        "exit_code": exit_code,
        "timed_out": timed_out,
        "wall_seconds": elapsed,
        "user_cpu_seconds": usage.ru_utime,
        "system_cpu_seconds": usage.ru_stime,
        "maximum_resident_set_bytes": usage.ru_maxrss * (1 if system == "Darwin" else 1024),
        "major_page_faults": usage.ru_majflt,
        "swaps": usage.ru_nswap,
        "source": "resource.getrusage(RUSAGE_CHILDREN), single command",
        "platform": system,
    }
    args.metrics.parent.mkdir(parents=True, exist_ok=True)
    args.metrics.write_text(json.dumps(metrics, indent=2) + "\n")
    return exit_code if exit_code >= 0 else 128 - exit_code


if __name__ == "__main__":
    sys.exit(main())
