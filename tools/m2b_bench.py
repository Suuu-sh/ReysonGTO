#!/usr/bin/env python3
"""Serial fixed-input M2b runner (stdlib only; never builds or alters ranges).

Example: python3 tools/m2b_bench.py --binary target/release/cli --out data/m2b/test
Matrix: --config matrix.json, containing {"defaults": {...}, "cases": [
  {"name":"turn-g2", "board":"QsJh2h3c", "gamma":2}, ...]}.
Config keys match long CLI options with underscores. Paths are relative to repo.
Each solve gets a fresh measure_command.py process, isolating its getrusage RSS
from telemetry commands and earlier solves. Telemetry runs outside that process.
"""
import argparse
import datetime
import hashlib
import json
import math
from pathlib import Path
import re
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
DEFAULTS = dict(binary='target/release/cli', board='QsJh2h3c', max_iters=1000,
                max_seconds=2250, timeout_seconds=2390, algorithm='dcfr',
                gamma=2.0, alpha=1.5, beta=0.0, threads=10, storage='i16',
                memory_mb=18000, metrics_every=10, telemetry_seconds=10.0,
                no_profiling=False, parallel_grain=1,
                adaptive_metrics=False, metrics_early_every=25,
                metrics_near_every=5, metrics_near_target_multiplier=2.0)


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def capture(command):
    try:
        p = subprocess.run(command, capture_output=True, text=True, timeout=4)
        return dict(command=command, exit_code=p.returncode, stdout=p.stdout, stderr=p.stderr)
    except (OSError, subprocess.TimeoutExpired) as error:
        return dict(command=command, error=str(error))


def telemetry(pid):
    ps = capture(['ps', '-axo', 'pid=,ppid=,rss=,%cpu=,comm='])
    # Retain only wrapper and its direct solver child; command names, not args.
    rows = ps.get('stdout', '').splitlines()
    ps['stdout'] = '\n'.join(row for row in rows if len(row.split()) >= 2
                             and str(pid) in row.split()[:2])
    return dict(timestamp_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                monotonic_seconds=time.monotonic(), measure_pid=pid, processes=ps,
                vm_stat=capture(['vm_stat']), swap=capture(['sysctl', 'vm.swapusage']),
                thermal=capture(['pmset', '-g', 'therm']))


def command_for(c, directory):
    cmd = [str((ROOT / c['binary']).resolve()), 'solve', '--board', c['board'],
           '--pot', '5.5', '--stack', '97.5', '--oop-range', 'data/ranges/bb_call.json',
           '--ip-range', 'data/ranges/btn_open.json', '--bet', '50', '--oop-leads',
           '--units-per-bb', '65536', '--raise-multiplier', '2.5', '--no-allin-merge',
           '--rake', '0', '--rake-cap', '0', '--target', '0.3']
    for key, flag in [('max_iters','max-iters'), ('max_seconds','max-seconds'),
                      ('algorithm','algorithm'), ('gamma','dcfr-gamma'),
                      ('alpha','dcfr-alpha'), ('beta','dcfr-beta'), ('threads','threads'),
                      ('storage','storage'), ('memory_mb','memory-mb'),
                      ('metrics_every','metrics-every')]:
        cmd += ['--' + flag, str(c[key])]
    if c['no_profiling']:
        cmd += ['--no-profiling']
    if c['parallel_grain'] != 1:
        cmd += ['--parallel-grain', str(c['parallel_grain'])]
    if c['adaptive_metrics']:
        cmd += ['--adaptive-metrics']
        for key in ('metrics_early_every', 'metrics_near_every', 'metrics_near_target_multiplier'):
            cmd += ['--' + key.replace('_', '-'), str(c[key])]
    return cmd + ['--progress-jsonl', str(directory / 'progress.jsonl'),
                  '--out', str(directory / 'solution.json')]


def validate(c):
    if c['board'] not in ('QsJh2h', 'QsJh2h3c'):
        raise ValueError('only fixed flop QsJh2h or turn QsJh2h3c supported')
    if c['algorithm'] not in ('dcfr', 'cfr-plus') or c['storage'] not in ('f32', 'i16'):
        raise ValueError('invalid algorithm or storage')
    for key in ('max_iters', 'threads', 'memory_mb', 'parallel_grain', 'metrics_every', 'metrics_early_every', 'metrics_near_every'):
        if not isinstance(c[key], int) or c[key] <= 0:
            raise ValueError(key + ' must be a positive integer')
    for key in ('max_seconds', 'timeout_seconds', 'telemetry_seconds', 'metrics_near_target_multiplier'):
        if not math.isfinite(c[key]) or c[key] <= 0:
            raise ValueError(key + ' must be positive and finite')
    for key in ('alpha', 'beta', 'gamma'):
        if not math.isfinite(c[key]):
            raise ValueError(key + ' must be finite')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path)
    parser.add_argument('--out', type=Path, required=True, help='new output directory, never overwritten')
    parser.add_argument('--dry-run', action='store_true')
    for key, value in DEFAULTS.items():
        kwargs = dict(action='store_true') if isinstance(value, bool) else dict(type=type(value))
        parser.add_argument('--' + key.replace('_', '-'), default=None, **kwargs)
    args = parser.parse_args()
    config = json.loads(args.config.read_text()) if args.config else {}
    if set(config) - {'defaults', 'cases'}:
        parser.error('config accepts only defaults and cases')
    overrides = {key: getattr(args, key) for key in DEFAULTS if getattr(args, key) is not None}
    cases = []
    for index, row in enumerate(config.get('cases', [{}])):
        c = DEFAULTS | config.get('defaults', {}) | row | overrides
        if set(c) - (set(DEFAULTS) | {'name'}):
            parser.error('unknown case settings: ' + str(set(c) - (set(DEFAULTS) | {'name'})))
        c.setdefault('name', f'{index:02d}-{c["board"]}')
        if not re.fullmatch(r'[A-Za-z0-9_-]+', c['name']):
            parser.error('case names must contain only letters, numbers, _ or -')
        validate(c)
        cases.append(c)
    if not cases or len({c['name'] for c in cases}) != len(cases):
        parser.error('cases must be nonempty with unique names')
    out = args.out.resolve()
    if args.dry_run:
        print(json.dumps([command_for(c, out / c['name']) for c in cases], indent=2))
        return 0
    # Validate all binaries and inputs before starting any solve.
    binaries = {c['binary']: digest((ROOT / c['binary']).resolve()) for c in cases}
    sources = sorted(set(ROOT.glob('crates/**/*.rs')) | set(ROOT.glob('crates/**/Cargo.toml')) |
                     {ROOT / 'Cargo.toml', ROOT / 'Cargo.lock', Path(__file__).resolve(), ROOT / 'tools/measure_command.py'})
    sources = [p for p in sources if 'target' not in p.relative_to(ROOT).parts]
    source_hashes = {str(p.relative_to(ROOT)): digest(p) for p in sources}
    manifest = dict(source_files_at_launch=source_hashes, working_tree_source_sha256=hashlib.sha256(
        json.dumps(source_hashes, sort_keys=True).encode()).hexdigest(), binaries=binaries,
        inputs={str(p): digest(ROOT / p) for p in ('data/ranges/bb_call.json', 'data/ranges/btn_open.json')},
        source_note='Working-tree snapshot at launch; not a claim that these sources built preserved binaries.',
        platform=capture(['uname', '-a']), cases=cases)
    out.mkdir(parents=True, exist_ok=False)
    (out / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    summaries = []
    for c in cases:
        directory = out / c['name']
        directory.mkdir()
        if digest((ROOT / c['binary']).resolve()) != binaries[c['binary']]:
            raise RuntimeError('binary changed during matrix')
        for path, expected in manifest['inputs'].items():
            if digest(ROOT / path) != expected:
                raise RuntimeError('range changed during matrix')
        cmd = command_for(c, directory)
        wrapped = [sys.executable, str(ROOT / 'tools/measure_command.py'), '--metrics',
                   str(directory / 'resource.json'), '--timeout-seconds', str(c['timeout_seconds']), '--'] + cmd
        (directory / 'command.json').write_text(json.dumps(wrapped, indent=2) + '\n')
        with (directory / 'stdout.log').open('w') as stdout, (directory / 'stderr.log').open('w') as stderr, (directory / 'telemetry.jsonl').open('w') as trace:
            process = subprocess.Popen(wrapped, cwd=ROOT, stdout=stdout, stderr=stderr)
            while True:
                trace.write(json.dumps(telemetry(process.pid)) + '\n')
                trace.flush()
                try:
                    code = process.wait(timeout=c['telemetry_seconds'])
                    final_sample = telemetry(process.pid)
                    final_sample['phase'] = 'after_process_exit'
                    trace.write(json.dumps(final_sample) + '\n')
                    trace.flush()
                    break
                except subprocess.TimeoutExpired:
                    pass
        record = dict(name=c['name'], settings=c, wrapper_exit_code=code)
        for filename, key in [('resource.json', 'resource'), ('solution.json', 'solution')]:
            path = directory / filename
            if path.exists():
                record[key] = json.loads(path.read_text())
        # Preserve complete solver results; no exit-code-to-target-success inference.
        summaries.append(record)
        (out / 'summary.json').write_text(json.dumps(summaries, indent=2) + '\n')
    return int(any(s['wrapper_exit_code'] != 0 for s in summaries))


if __name__ == '__main__':
    sys.exit(main())
