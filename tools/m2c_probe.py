#!/usr/bin/env python3
"""One fixed-input M2c solve, optional exact-PID macOS sampling (stdlib only).

Never builds, changes ranges, or starts a matrix. Use a new --out directory.
The binary is run directly; only wait4(binary_pid) supplies solver resources.
Source hashes are a launch snapshot, not proof of binary build provenance.
"""
import argparse
import datetime
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import signal
import subprocess
import sys
import threading
import time

from m2b_bench import ROOT, DEFAULTS, capture, command_for, digest, telemetry


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def trace_os(pid, path, stop):
    with path.open('w') as stream:
        while not stop.is_set():
            try:
                row = telemetry(pid)
                row['solver_pid'] = pid
                row['phase'] = 'running'
            except Exception as error:
                row = {'error': repr(error), 'solver_pid': pid}
            stream.write(json.dumps(row) + '\n')
            stream.flush()
            stop.wait(10)
        row = telemetry(pid)
        row['solver_pid'] = pid
        row['phase'] = 'after_process_exit'
        stream.write(json.dumps(row) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--board', choices=['QsJh2h', 'QsJh2h3c'], default='QsJh2h')
    parser.add_argument('--threads', type=int, default=10)
    parser.add_argument('--max-iters', type=int, default=1000)
    parser.add_argument('--max-seconds', type=float, default=2250)
    parser.add_argument('--timeout-seconds', type=float, default=2390)
    parser.add_argument('--metrics-every', type=int, default=10)
    parser.add_argument('--adaptive-metrics', action='store_true')
    parser.add_argument('--kernel-diagnostics', action='store_true')
    parser.add_argument('--sample-seconds', type=int, default=0,
                        help='sample duration; 0 disables sampling')
    parser.add_argument('--sample-delay', type=float, default=25)
    args = parser.parse_args()
    for key in ('threads', 'max_iters', 'metrics_every'):
        if getattr(args, key) <= 0:
            parser.error(key.replace('_', '-') + ' must be positive')
    for key in ('max_seconds', 'timeout_seconds'):
        if not math.isfinite(getattr(args, key)) or getattr(args, key) <= 0:
            parser.error(key.replace('_', '-') + ' must be positive and finite')
    if args.sample_seconds < 0 or not math.isfinite(args.sample_delay) or args.sample_delay < 0:
        parser.error('sampling duration and delay must be nonnegative and finite')
    if not hasattr(os, 'wait4'):
        parser.error('this runner requires os.wait4')
    system = platform.system()
    if system not in ('Darwin', 'Linux'):
        parser.error('unknown ru_maxrss units on ' + system)
    binary = (ROOT / args.binary).resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        parser.error('binary must be an existing executable')
    out = args.out.resolve()
    c = DEFAULTS | dict(binary=str(binary), board=args.board, gamma=4.0,
                        threads=args.threads, max_iters=args.max_iters,
                        max_seconds=args.max_seconds, timeout_seconds=args.timeout_seconds,
                        metrics_every=args.metrics_every, adaptive_metrics=args.adaptive_metrics)
    command = command_for(c, out)
    if args.kernel_diagnostics:
        command.append('--kernel-diagnostics')
    inputs = {p: digest(ROOT / p) for p in
              ('data/ranges/bb_call.json', 'data/ranges/btn_open.json',
               'data/benchmark/tree-preflight.json')}
    sources = sorted(set(ROOT.glob('crates/**/*.rs')) |
                     set(ROOT.glob('crates/**/Cargo.toml')) |
                     {ROOT / 'Cargo.toml', ROOT / 'Cargo.lock', Path(__file__).resolve(),
                      ROOT / 'tools/m2b_bench.py'})
    hashes = {str(p.relative_to(ROOT)): digest(p) for p in sources
              if 'target' not in p.relative_to(ROOT).parts}
    manifest = dict(timestamp_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                    command=command, settings=c, probe_settings=vars(args) | {'out': str(out)},
                    binary=str(binary), binary_sha256=digest(binary), inputs=inputs,
                    source_files_at_launch=hashes,
                    working_tree_source_sha256=hashlib.sha256(
                        json.dumps(hashes, sort_keys=True).encode()).hexdigest(),
                    source_note='Launch working-tree snapshot only; not binary build provenance.',
                    platform=capture(['uname', '-a']))
    out.mkdir(parents=True, exist_ok=False)
    save(out / 'manifest.json', manifest)
    save(out / 'command.json', command)
    stop = threading.Event()
    process = sample = None
    worker = None
    sample_stdout = sample_stderr = None
    usage = status = None
    timed_out = False
    error = None
    start = time.monotonic()
    elapsed = None
    sample_record = dict(requested_seconds=args.sample_seconds, delay_seconds=args.sample_delay,
                         started=False)
    try:
        with (out / 'stdout.log').open('w') as stdout, (out / 'stderr.log').open('w') as stderr:
            start = time.monotonic()
            process = subprocess.Popen(command, cwd=ROOT, stdout=stdout, stderr=stderr)
            (out / 'child.pid').write_text(str(process.pid) + '\n')
            worker = threading.Thread(target=trace_os,
                                      args=(process.pid, out / 'telemetry.jsonl', stop), daemon=True)
            worker.start()
            while True:
                pid, new_status, new_usage = os.wait4(process.pid, os.WNOHANG)
                if pid:
                    status, usage = new_status, new_usage
                    process.returncode = os.waitstatus_to_exitcode(status)
                    elapsed = time.monotonic() - start
                    break
                now = time.monotonic()
                if now - start >= args.timeout_seconds:
                    timed_out = True
                    try:
                        os.kill(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    _, status, usage = os.wait4(process.pid, 0)
                    process.returncode = os.waitstatus_to_exitcode(status)
                    elapsed = time.monotonic() - start
                    break
                if args.sample_seconds and not sample_record['started'] and now - start >= args.sample_delay:
                    sample_record.update(started=True, solver_pid=process.pid,
                                         elapsed_at_start_seconds=now - start)
                    sample_command = ['/usr/bin/sample', str(process.pid), str(args.sample_seconds),
                                      '10', '-mayDie', '-fullPaths', '-file', str(out / 'sample.txt')]
                    sample_record['command'] = sample_command
                    sample_stdout = (out / 'sample-stdout.log').open('w')
                    sample_stderr = (out / 'sample-stderr.log').open('w')
                    try:
                        sample = subprocess.Popen(sample_command, stdout=sample_stdout,
                                                  stderr=sample_stderr)
                    except OSError as exc:
                        sample_record['error'] = repr(exc)
                        sample_stderr.write(str(exc) + '\n')
                        sample_stderr.flush()
                time.sleep(0.1)
    except BaseException as exc:
        error = repr(exc)
        raise
    finally:
        # Never use Popen.poll/wait for the solver: either can reap it without rusage.
        if process is not None and process.returncode is None:
            try:
                os.kill(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            _, status, usage = os.wait4(process.pid, 0)
            process.returncode = os.waitstatus_to_exitcode(status)
            elapsed = time.monotonic() - start
        stop.set()
        if sample is not None:
            if sample.poll() is None:
                sample_record['terminated_at_solver_exit'] = True
                sample.terminate()
            try:
                sample.wait(timeout=5)
            except subprocess.TimeoutExpired:
                sample.kill()
                sample.wait()
            sample_record['exit_code'] = sample.returncode
        for stream in (sample_stdout, sample_stderr):
            if stream is not None:
                stream.close()
        if worker is not None:
            worker.join(timeout=25)
        save(out / 'sample.json', sample_record)
        if usage is not None:
            code = 124 if timed_out else process.returncode
            resource = dict(command=command, solver_pid=process.pid, exit_code=code,
                            child_exit_code=process.returncode, timed_out=timed_out,
                            wall_seconds=elapsed, user_cpu_seconds=usage.ru_utime,
                            system_cpu_seconds=usage.ru_stime,
                            maximum_resident_set_bytes=usage.ru_maxrss * (1 if system == 'Darwin' else 1024),
                            major_page_faults=usage.ru_majflt, swaps=usage.ru_nswap,
                            source='os.wait4(exact solver PID); excludes sampler and telemetry children',
                            platform=system, error=error)
            save(out / 'resource.json', resource)
        elif error is not None:
            save(out / 'resource.json', dict(error=error, timed_out=False))
    return code if code >= 0 else 128 - code


if __name__ == '__main__':
    sys.exit(main())
