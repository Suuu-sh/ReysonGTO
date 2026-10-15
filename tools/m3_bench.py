#!/usr/bin/env python3
"""Sequential M3 production-rule benchmark (stdlib only; never builds).

Defaults: Qh8d2c, As7d2c, Th9h8c; original external BB/BTN ranges; f32,
DCFR gamma=2, 33/66/66 sizing, rake 5% cap 3BB, no flop donk, 2.5x raises,
67% all-in merge, 10 threads. Use a NEW --out directory. No range is edited.
The soft 1740s limit reserves time for final exact metrics under the independent
1800s watchdog; setup/final metrics can still cause a hard timeout. The watchdog
polls every 0.1s, so observed wall time can slightly exceed its deadline.
RSS comes from wait4(exact solver PID), not cumulative previous children.
Source hashes describe launch state, NOT proof of preserved binary provenance.
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
import time

ROOT = Path(__file__).resolve().parents[1]
BOARDS = ('Qh8d2c', 'As7d2c', 'Th9h8c')


def digest(path):
    result = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            result.update(block)
    return result.hexdigest()


def save(path, value):
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + '\n')


def capture(command):
    try:
        process = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=4)
        return dict(command=command, exit_code=process.returncode,
                    stdout=process.stdout.strip(), stderr=process.stderr.strip())
    except (OSError, subprocess.TimeoutExpired) as error:
        return dict(command=command, error=str(error))


def command_for(args, board, directory):
    return [str(args.binary), 'solve', '--preset', 'production', '--board', board,
            '--oop-range', str(args.oop_range), '--ip-range', str(args.ip_range),
            '--pot', '5.5', '--stack', '97.5', '--units-per-bb', '1000',
            '--bet-flop', '33', '--bet-turn', '66', '--bet-river', '66',
            '--no-oop-leads', '--raise-multiplier', '2.5', '--allin-merge',
            '--allin-merge-ratio', '0.67', '--rake', '0.05', '--rake-cap', '3',
            '--storage', 'f32', '--algorithm', 'dcfr',
            '--dcfr-alpha', '1.5', '--dcfr-beta', '0', '--dcfr-gamma', '2',
            '--threads', str(args.threads), '--memory-mb', str(args.memory_mb),
            '--target', str(args.target), '--max-iters', str(args.max_iters),
            '--max-seconds', str(args.max_seconds),
            '--adaptive-metrics', '--metrics-early-every', '25',
            '--metrics-near-every', '5', '--metrics-near-target-multiplier', '2',
            '--progress-jsonl', str(directory / 'progress.jsonl'),
            '--out', str(directory / 'solution.json')]


def run_child(command, directory, timeout_seconds):
    """One direct child, killed AND reaped before returning or raising."""
    process = usage = None
    timed_out = False
    start = time.monotonic()
    try:
        with (directory / 'stdout.log').open('w') as stdout, (directory / 'stderr.log').open('w') as stderr:
            process = subprocess.Popen(command, cwd=ROOT, stdout=stdout, stderr=stderr)
            (directory / 'child.pid').write_text(str(process.pid) + '\n')
            while True:
                pid, status, usage_candidate = os.wait4(process.pid, os.WNOHANG)
                if pid:
                    usage = usage_candidate
                    process.returncode = os.waitstatus_to_exitcode(status)
                    break
                if time.monotonic() - start >= timeout_seconds:
                    timed_out = True
                    break
                time.sleep(0.1)
    finally:
        # Do not use Popen.poll/wait: they can reap without the exact PID rusage.
        if process is not None and process.returncode is None:
            try:
                os.kill(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            _, status, usage = os.wait4(process.pid, 0)
            process.returncode = os.waitstatus_to_exitcode(status)
    if usage is None:
        raise RuntimeError('child resource usage unavailable')
    system = platform.system()
    return dict(command=command, solver_pid=process.pid,
                exit_code=124 if timed_out else process.returncode,
                child_exit_code=process.returncode, timed_out=timed_out,
                wall_seconds=time.monotonic() - start,
                user_cpu_seconds=usage.ru_utime, system_cpu_seconds=usage.ru_stime,
                maximum_resident_set_bytes=usage.ru_maxrss * (1 if system == 'Darwin' else 1024),
                major_page_faults=usage.ru_majflt, swaps=usage.ru_nswap,
                source='os.wait4(exact solver PID); excludes prior solves and manifest commands',
                platform=system)


def checkpoints(path):
    records, errors = [], []
    if path.exists():
        for line_number, line in enumerate(path.read_text().splitlines(), 1):
            try:
                event = json.loads(line)
                if event.get('phase') == 'checkpoint':
                    records.append(event['record'])
            except (ValueError, TypeError, KeyError, AttributeError) as error:
                errors.append(dict(line=line_number, error=str(error)))
    return records, errors


def btn_first_strategy(report, board):
    strategy = report['after_check_strategy']
    if not isinstance(strategy, dict) or strategy.get('player') != 1:
        raise ValueError('missing BTN first decision after forced OOP check')
    actions = strategy['actions']
    bets = [i for i, action in enumerate(actions) if isinstance(action, dict) and 'Bet' in action]
    if actions.count('Check') != 1 or len(bets) != 1 or len(actions) != 2:
        raise ValueError('BTN first decision must have exactly Check and one Bet')
    rows = strategy['combos']
    if not rows:
        raise ValueError('BTN first decision has no combos')
    frequencies = {}
    for name, key in [('compatible_joint', 'reach_weight'), ('own_reach', 'weight')]:
        sums = [0.0] * len(actions)
        total = 0.0
        for row in rows:
            weight = row[key]
            probabilities = row['probabilities']
            if not isinstance(weight, (float, int)) or not math.isfinite(weight) or weight < 0:
                raise ValueError('invalid strategy weight')
            if (len(probabilities) != len(actions)
                    or any(not math.isfinite(p) or p < 0 or p > 1.000001 for p in probabilities)
                    or abs(sum(probabilities) - 1.0) > 1e-5):
                raise ValueError('invalid strategy probabilities')
            total += weight
            for i, probability in enumerate(probabilities):
                sums[i] += weight * probability
        if total <= 0 or not math.isfinite(total):
            raise ValueError('strategy has zero or invalid total weight')
        frequencies[name] = dict(total_weight=total,
                                 action_probabilities=[value / total for value in sums],
                                 bet_frequency_pct=100.0 * sums[bets[0]] / total)
    return dict(board=board, node=strategy['node'], player=1, actions=actions,
                classes=strategy['classes'], combo_count=len(rows),
                action_frequencies=frequencies, primary_weighting='compatible_joint',
                class_weighting='compatible_joint (solver-exported classes)',
                own_reach_note='Input BTN combo weight: its first decision has no prior BTN action.',
                source='solution.json after_check_strategy; no copied-reference comparison')


def summarize(board, directory, resource, target):
    records, errors = checkpoints(directory / 'progress.jsonl')
    path = directory / 'solution.json'
    summary = dict(board=board, directory=str(directory), resource=resource,
                   solution_present=path.is_file(), final_result_accepted=False,
                   final_exploitability_pct=None, target_met=None, iterations=None,
                   setup_seconds=None, solve_seconds=None, training_seconds=None,
                   metrics_seconds=None, stopped_by=None, btn_bet_frequency_pct=None,
                   btn_bet_frequency_own_reach_pct=None,
                   checkpoints=records, last_checkpoint=records[-1] if records else None,
                   progress_parse_errors=errors)
    if resource.get('timed_out'):
        summary['status'] = 'timeout'
        summary['stopped_by'] = 'hard_timeout'
    elif resource.get('exit_code') != 0:
        summary['status'] = 'process_error'
        summary['stopped_by'] = 'process_error'
    elif not path.is_file():
        summary['status'] = 'missing_result'
        summary['stopped_by'] = 'missing_result'
    else:
        try:
            report = json.loads(path.read_text())
            accuracy = report['metrics']['exploitability_pct']
            if not isinstance(accuracy, (float, int)) or not math.isfinite(accuracy) or accuracy < -1e-8:
                raise ValueError('invalid final exploitability')
            strategy = btn_first_strategy(report, board)
            final = {key: report[key] for key in ('iterations', 'setup_seconds', 'solve_seconds',
                                                  'training_seconds', 'metrics_seconds', 'stopped_by')}
            save(directory / 'btn-first-strategy.json', strategy)
            summary.update(final, final_result_accepted=True, final_exploitability_pct=accuracy,
                           target_met=accuracy <= target, status='complete',
                           btn_bet_frequency_pct=strategy['action_frequencies']['compatible_joint']['bet_frequency_pct'],
                           btn_bet_frequency_own_reach_pct=strategy['action_frequencies']['own_reach']['bet_frequency_pct'])
        except (ValueError, TypeError, KeyError, OSError, AttributeError) as error:
            summary.update(status='invalid_result', stopped_by='invalid_result', result_error=str(error))
    save(directory / 'summary.json', summary)
    return summary


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/cli')
    parser.add_argument('--oop-range', type=Path, default=ROOT / 'data/ranges/bb_call.json')
    parser.add_argument('--ip-range', type=Path, default=ROOT / 'data/ranges/btn_open.json')
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--boards', nargs='+', choices=BOARDS, default=list(BOARDS))
    parser.add_argument('--threads', type=int, default=10)
    parser.add_argument('--memory-mb', type=int, default=16384)
    parser.add_argument('--target', type=float, default=0.3)
    parser.add_argument('--max-iters', type=int, default=10000)
    parser.add_argument('--max-seconds', type=float, default=1740)
    parser.add_argument('--timeout-seconds', type=float, default=1800)
    args = parser.parse_args(argv)
    for key in ('threads', 'memory_mb', 'max_iters'):
        if getattr(args, key) <= 0:
            parser.error(key.replace('_', '-') + ' must be positive')
    for key in ('target', 'max_seconds', 'timeout_seconds'):
        if not math.isfinite(getattr(args, key)) or getattr(args, key) <= 0:
            parser.error(key.replace('_', '-') + ' must be positive and finite')
    if args.max_seconds >= args.timeout_seconds:
        parser.error('soft max-seconds must be below hard timeout-seconds')
    if len(set(args.boards)) != len(args.boards):
        parser.error('boards must be unique')
    if not hasattr(os, 'wait4') or platform.system() not in ('Darwin', 'Linux'):
        parser.error('requires wait4 and known Darwin/Linux RSS units')
    args.out = args.out.resolve()
    if args.out.exists():
        parser.error('output directory already exists; refusing overwrite')
    for key in ('binary', 'oop_range', 'ip_range'):
        path = getattr(args, key).resolve()
        if not path.is_file() or (key == 'binary' and not os.access(path, os.X_OK)):
            parser.error(key.replace('_', '-') + ' must be an existing ' + ('executable' if key == 'binary' else 'file'))
        setattr(args, key, path)
    source_files = sorted(set(ROOT.glob('crates/**/*.rs')) | set(ROOT.glob('crates/**/Cargo.toml'))
                          | {ROOT / 'Cargo.toml', ROOT / 'Cargo.lock', Path(__file__).resolve()})
    hashes = {str(path.relative_to(ROOT)): digest(path) for path in source_files
              if 'target' not in path.relative_to(ROOT).parts}
    commands = [command_for(args, board, args.out / f'{i:02d}-{board}')
                for i, board in enumerate(args.boards)]
    manifest = dict(timestamp_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                    settings={key: str(value) if isinstance(value, Path) else value
                              for key, value in vars(args).items()},
                    production_rules=dict(pot_bb=5.5, stack_bb=97.5, units_per_bb=1000,
                                          bet_percentages=[33, 66, 66], rake_rate=0.05, rake_cap_bb=3,
                                          oop_can_lead_flop=False, raise_multiplier=2.5,
                                          allin_merge_ratio=0.67, storage='f32', dcfr_gamma=2,
                                          adaptive_metrics=dict(early_every=25, near_every=5, near_target_multiplier=2)),
                    commands=commands, binary=str(args.binary), binary_sha256=digest(args.binary),
                    inputs={str(path): digest(path) for path in (args.oop_range, args.ip_range)},
                    source_files_at_launch=hashes,
                    working_tree_source_sha256=hashlib.sha256(json.dumps(hashes, sort_keys=True).encode()).hexdigest(),
                    source_note='Launch working-tree snapshot only; NOT binary build provenance.',
                    watchdog_note='0.1s polling; hard timeout kills and reaps direct solver PID before next board.',
                    git_head=capture(['git', 'rev-parse', 'HEAD']),
                    git_status=capture(['git', 'status', '--short']), platform=platform.platform())
    args.out.mkdir(parents=True, exist_ok=False)
    save(args.out / 'manifest.json', manifest)
    summaries = []
    for i, (board, command) in enumerate(zip(args.boards, commands)):
        directory = args.out / f'{i:02d}-{board}'
        directory.mkdir()
        save(directory / 'command.json', command)
        print(f'{board}: starting serial solve; soft {args.max_seconds}s, hard {args.timeout_seconds}s', flush=True)
        try:
            resource = run_child(command, directory, args.timeout_seconds)
        except OSError as error:
            resource = dict(exit_code=None, timed_out=False, error=str(error))
        save(directory / 'resource.json', resource)
        summary = summarize(board, directory, resource, args.target)
        summaries.append(summary)
        save(args.out / 'summary.json', dict(boards=summaries, all_complete=len(summaries) == len(args.boards)
                                           and all(row['final_result_accepted'] for row in summaries),
                                           all_targets_met=len(summaries) == len(args.boards)
                                           and all(row['target_met'] is True for row in summaries)))
        print(f'{board}: {summary["status"]}; final exploitability={summary["final_exploitability_pct"]}', flush=True)
    return 0 if all(row['final_result_accepted'] for row in summaries) else 1


if __name__ == '__main__':
    sys.exit(main())
