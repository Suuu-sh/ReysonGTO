#!/usr/bin/env python3
"""Serial M4 sizing pilot. No build, no range edits; diagnostic EV comparisons only."""
import argparse
import datetime
import hashlib
import json
import math
import os
from pathlib import Path
import sys
import time

import m3_bench as m3

ROOT = Path(__file__).resolve().parents[1]
SIZES = (33, 66, 100, 125)
POT = 5.5


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + '.tmp')
    temporary.write_text(json.dumps(value, indent=2, allow_nan=False) + '\n')
    os.replace(temporary, path)


def fingerprint(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


def diagnostic(a, b):
    gap = abs(a['btn_ev_bb'] - b['btn_ev_bb'])
    proxy = a['diagnostic_proxy_bb'] + b['diagnostic_proxy_bb']
    return dict(gap_bb=gap, gap_pot_pct=100 * gap / POT,
                combined_proxy_bb=proxy, indistinguishable_by_proxy=gap <= proxy,
                interpretation='heuristic diagnostic, NOT an equilibrium EV bound in raked general-sum play')


def metrics(path, target):
    report = json.loads(path.read_text())
    raw = report['metrics']
    ev, br = raw['average_ev'], raw['best_response_ev']
    exploit = raw['exploitability_pct']
    if len(ev) != 2 or len(br) != 2:
        raise ValueError('expected two player root EVs')
    values = [*ev, *br, exploit]
    if any(isinstance(x, bool) or not isinstance(x, (int, float)) or not math.isfinite(x) for x in values):
        raise ValueError('nonfinite or nonnumeric exact metrics')
    improvements = [(br[p] - ev[p]) / 1000 for p in range(2)]
    if exploit < 0 or min(improvements) < -1e-7:
        raise ValueError('negative exploitability / BR improvement')
    if abs(sum(improvements) / 2 / POT * 100 - exploit) > 1e-5:
        raise ValueError('inconsistent exploitability units')
    return dict(btn_ev_bb=ev[1] / 1000, btn_ev_pot_pct=ev[1] / 1000 / POT * 100,
                average_ev_bb=[x / 1000 for x in ev], best_response_ev_bb=[x / 1000 for x in br],
                br_improvement_bb=improvements, exploitability_pct=exploit,
                diagnostic_proxy_bb=2 * exploit * POT / 100,
                target_met=exploit <= target, exact_metrics=raw,
                iterations=report['iterations'], solve_seconds=report['solve_seconds'],
                stopped_by=report['stopped_by'])


def validate_solution(path, board, size, args):
    report = json.loads(path.read_text())
    spec = report['game_spec']
    cards = ['23456789TJQKA'.index(board[i]) * 4 + 'cdhs'.index(board[i + 1]) for i in range(0, 6, 2)]
    expected = dict(starting_pot=5500, effective_stack=97500, oop_can_lead_flop=False,
                    bet_size={'PerStreet': dict(flop=size / 100, turn=.66, river=.66)},
                    raise_multiplier=2.5, allin_merge_enabled=True, allin_merge_ratio=.67,
                    rake_rate=.05, rake_cap=3000, allow_raises=True)
    if spec['board'] != cards or spec['units_per_bb'] != 1000 or spec['tree'] != expected:
        raise ValueError('solution game specification differs from requested production board/size')
    engine = spec['engine']
    for key, value in dict(algorithm='Dcfr', threads=args.threads, storage_mode='F32', dcfr_alpha=1.5, dcfr_beta=0., dcfr_gamma=2.).items():
        if engine[key] != value:
            raise ValueError('solution engine configuration mismatch: ' + key)
    strategy = report['after_check_strategy']
    if strategy['player'] != 1 or strategy['actions'] != ['Check', {'Bet': int(5500 * size / 100 + .5)}]:
        raise ValueError('wrong BTN after-check actions')
    if report['root_strategy']['actions'] != ['Check'] or report['root_ev_chips'] != report['metrics']['average_ev']:
        raise ValueError('wrong root decision or root EV mismatch')


def reusable(record, identity):
    if record.get('identity') != identity or record.get('status') != 'complete':
        return False
    try:
        return record['solution_sha256'] == m3.digest(Path(record['solution_path']))
    except (OSError, KeyError):
        return False


def imported_stage1(args, board, size, stage):
    """A complete stage-1 result from an earlier run with the same game settings.

    Source hashes differ between runs (new crates), so only the solved game is compared:
    the solution file must still match its recorded digest and use the same stage-1 target."""
    if stage != 1 or args.reuse_stage1_from is None:
        return None
    path = args.reuse_stage1_from / 'runs' / board / f'stage1-{size}' / 'result.json'
    if not path.exists():
        return None
    record = json.loads(path.read_text())
    old = json.loads((args.reuse_stage1_from / 'manifest.json').read_text())['provenance']['config']
    same_game = all(old.get(k) == v for k, v in game_config().items())
    try:
        intact = record.get('solution_sha256') == m3.digest(Path(record['solution_path']))
    except (OSError, KeyError):
        intact = False
    if record.get('status') == 'complete' and same_game and intact and record.get('target_pct') == args.stage1_target:
        return dict(record, reused_from=str(path))
    return None


def game_config():
    # Lists, not tuples, so values compare equal to configs read back from JSON.
    return dict(sizes=list(SIZES), pot=POT, stack=97.5, player=1, turn=66, river=66, preset='production')


def candidate(args, board, size, stage, deadline, identity):
    directory = args.out / 'runs' / board / f'stage{stage}-{size}'
    directory.mkdir(parents=True, exist_ok=True)
    cached = directory / 'result.json'
    if cached.exists():
        record = json.loads(cached.read_text())
        if reusable(record, identity):
            return record
    imported = imported_stage1(args, board, size, stage)
    if imported:
        save(cached, dict(imported, identity=identity))
        print(f'{board} stage{stage} {size}%: reused from {args.reuse_stage1_from}', flush=True)
        return dict(imported, identity=identity)
    # Retain every failed attempt and partial checkpoint, never overwrite it on resume.
    attempt = 1
    while (directory / f'attempt-{attempt}').exists():
        attempt += 1
    directory = directory / f'attempt-{attempt}'
    directory.mkdir()
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        return dict(status='budget_exhausted', size=size, stage=stage, identity=identity)
    target = args.stage1_target if stage == 1 else args.stage2_target
    args.target, args.max_seconds = target, max(0.001, remaining - min(30, remaining / 10))
    command = m3.command_for(args, board, directory)
    command[command.index('--bet-flop') + 1] = str(size)
    resource = m3.run_child(command, directory, remaining)
    record = dict(size=size, stage=stage, target_pct=target, identity=identity,
                  resource=resource, solution_path=str(directory / 'solution.json'))
    records, errors = m3.checkpoints(directory / 'progress.jsonl')
    record.update(checkpoints=records, progress_parse_errors=errors)
    try:
        if resource['timed_out'] or resource['exit_code'] != 0:
            raise ValueError('timeout' if resource['timed_out'] else 'process_error')
        validate_solution(directory / 'solution.json', board, size, args)
        record.update(metrics(directory / 'solution.json', target), status='complete',
                      solution_sha256=m3.digest(directory / 'solution.json'))
    except (OSError, ValueError, KeyError, TypeError) as error:
        record.update(status='invalid_or_incomplete', error=str(error))
    save(directory / 'result.json', record)
    save(cached, record)
    print(f'{board} stage{stage} {size}%: {record["status"]}, EV={record.get("btn_ev_bb")}, exploit%={record.get("exploitability_pct")}', flush=True)
    return record


def board_result(selection, stage1, stage2, identity):
    latest = {r['size']: r for r in stage1 if r['status'] == 'complete'}
    latest.update({r['size']: r for r in stage2 if r['status'] == 'complete'})
    ranked = sorted(latest.values(), key=lambda r: (-r['btn_ev_bb'], r['size']))
    complete = len(stage1) == 4 and len(stage2) == 2 and all(r['status'] == 'complete' for r in stage1 + stage2)
    refined = sorted([r for r in stage2 if r['status'] == 'complete'], key=lambda r: (-r['btn_ev_bb'], r['size']))
    winner = refined[0] if refined else None
    comparisons = []
    if winner:
        for row in ranked:
            if row['size'] == winner['size']:
                continue
            comparisons.append(dict(size=row['size'], eliminated_unrefined=row['stage'] == 1,
                                    **diagnostic(winner, row)))
    return dict(**selection, identity=identity, complete=complete, stage1=stage1, stage2=stage2,
                observed_highest_ev_size=winner['size'] if winner else None,
                mixed_precision_highest_ev_size=ranked[0]['size'] if ranked else None,
                comparisons=comparisons, no_optimality_claim=True)


def summarize(out, report):
    manifest = json.loads((out / 'manifest.json').read_text())
    rows = []
    for selected in manifest['selection']['training']:
        path = out / 'flops' / (selected['flop'] + '.json')
        rows.append(json.loads(path.read_text()) if path.exists() else dict(**selected, complete=False, stage1=[], stage2=[]))
    lines = ['# M4 sizing trial (pilot)', '',
             'BTN/IP player 1 root game EV, including check; pot 5.5 BB, stack 97.5 BB. Production rake 5% cap 3 BB; flop-only sizing varies, turn/river 66%.',
             'All four sizes at 5%, then observed top two at 2% (fresh solves). Coarse eliminated candidates remain unrefined.',
             '**No confident optimal-size claim.** Raked general-sum exploitability is not a rigorous equilibrium-EV bound. Per-candidate diagnostic proxy = 2 × exploitability% × 5.5 / 100 BB; gap comparison sums both proxies. This is a conservative heuristic diagnostic only.',
             '20 training flops, validation 0, seed 20261015: limited feature coverage, not the full 61 strata or held-out validation.', '',
             '| Flop / features | Stage | Size % | BTN EV BB / pot % | Exploit % / target met | BR improvements BB (OOP, IP) | Wall sec / RSS bytes |',
             '|---|---:|---:|---:|---|---|---|']
    for row in rows:
        for r in row['stage1'] + row['stage2']:
            if r['status'] != 'complete':
                lines.append(f"| {row['flop']} | {r['stage']} | {r['size']} | incomplete: {r['status']} | — | — | — |")
                continue
            lines.append(f"| {row['flop']} {json.dumps(row['features'], sort_keys=True)} | {r['stage']} | {r['size']} | {r['btn_ev_bb']:.9f} / {r['btn_ev_pot_pct']:.9f} | {r['exploitability_pct']:.9f} / {r['target_met']} | {r['br_improvement_bb']} | {r['resource']['wall_seconds']:.3f} / {r['resource']['maximum_resident_set_bytes']} |")
    lines += ['', '## Board summary (refined survivors only)',
              '| Flop | Observed winner / runner-up | Gap BB / combined proxy BB | Diagnostic | Total wall seconds |',
              '|---|---|---|---|---|']
    for row in rows:
        refined = sorted([r for r in row['stage2'] if r['status'] == 'complete'], key=lambda r: (-r['btn_ev_bb'], r['size']))
        wall = sum(r.get('resource', {}).get('wall_seconds', 0) for r in row['stage1'] + row['stage2'])
        if len(refined) == 2:
            comparison = diagnostic(*refined)
            label = '区別できない' if comparison['indistinguishable_by_proxy'] else 'proxy exceeded; still not an optimality proof'
            lines.append(f"| {row['flop']} | {refined[0]['size']} / {refined[1]['size']} | {comparison['gap_bb']:.9f} / {comparison['combined_proxy_bb']:.9f} | {label} | {wall:.3f} |")
        else:
            lines.append(f"| {row['flop']} | incomplete | — | 区別できない | {wall:.3f} |")
        if row.get('mixed_precision_highest_ev_size') != row.get('observed_highest_ev_size'):
            lines.append(f"Mixed-precision apparent highest for {row['flop']}: {row.get('mixed_precision_highest_ev_size')}%; not a refined winner.")
    lines += ['', '## Gaps and cautious grouped trends']
    groups = {}
    for row in rows:
        if row.get('observed_highest_ev_size') is not None:
            key = json.dumps(row['features'], sort_keys=True)
            groups.setdefault(key, []).append(f"{row['flop']}: observed {row['observed_highest_ev_size']}%")
        for c in row.get('comparisons', []):
            lines.append(f"- {row['flop']} vs {c['size']}%: gap {c['gap_bb']:.9f} BB ({c['gap_pot_pct']:.9f}% pot), combined proxy {c['combined_proxy_bb']:.9f} BB; indistinguishable={c['indistinguishable_by_proxy']}; eliminated/unrefined={c['eliminated_unrefined']}.")
    for key, values in groups.items():
        lines.append(f'- Features {key}: ' + '; '.join(values) + '. Descriptive only; no learned rule or optimality inference.')
    lines += ['', '## Incomplete boards', ', '.join(r['flop'] for r in rows if not r['complete']) or 'None.',
              '', '## Reproducibility', 'Manifest includes binary, source, configuration and original input-file hashes (no range contents). Serial child watchdog kills and reaps; checkpoint and failed attempts retained.',
              '```json', json.dumps(manifest, indent=2), '```']
    report.parent.mkdir(parents=True, exist_ok=True)
    temporary = report.with_suffix('.tmp')
    temporary.write_text('\n'.join(lines) + '\n')
    os.replace(temporary, report)
    save(out / 'summary.json', dict(completed=sum(r['complete'] for r in rows), total=len(rows), boards=rows))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/cli')
    parser.add_argument('--sizing-binary', type=Path, default=ROOT / 'target/release/sizing')
    parser.add_argument('--out', type=Path, default=ROOT / 'data/m4')
    parser.add_argument('--report', type=Path, default=ROOT / 'docs/benchmarks/m4-trial.md')
    parser.add_argument('--threads', type=int, default=10)
    parser.add_argument('--memory-mb', type=int, default=16384)
    parser.add_argument('--max-hours', type=float, default=8)
    parser.add_argument('--max-iters', type=int, default=10000)
    parser.add_argument('--summarize-only', action='store_true')
    parser.add_argument('--stage1-target', type=float, default=5.0)
    parser.add_argument('--stage2-target', type=float, default=2.0)
    parser.add_argument('--reuse-stage1-from', type=Path, default=None,
                        help='earlier --out directory whose complete stage-1 solves are reused')
    parser.add_argument('--limit', type=int, default=20, help='solve only the first N selected training boards')
    parser.add_argument('--training', type=int, default=20, help='representative boards to select (fixed per --out)')
    parser.add_argument('--validation', type=int, default=0, help='check boards to select and solve after the training boards')
    parser.add_argument('--shard', default='0/1', help='i/n: solve only boards with index %% n == i')
    parser.add_argument('--prepare-only', action='store_true', help='write the manifest (board selection) and exit; run once before starting shards')
    args = parser.parse_args(argv)
    if not math.isfinite(args.max_hours) or not 0 < args.max_hours <= 48 or min(args.threads, args.memory_mb, args.max_iters, args.limit) <= 0:
        parser.error('positive resources required; max-hours must be <= 48')
    try:
        args.shard_index, args.shard_count = (int(x) for x in args.shard.split('/'))
    except ValueError:
        parser.error('--shard must look like i/n')
    if not 0 <= args.shard_index < args.shard_count:
        parser.error('--shard needs 0 <= i < n')
    if not 0 < args.stage2_target <= args.stage1_target:
        parser.error('stage targets must satisfy 0 < stage2 <= stage1')
    if args.reuse_stage1_from is not None:
        args.reuse_stage1_from = args.reuse_stage1_from.resolve()
    args.out, args.report = args.out.resolve(), args.report.resolve()
    if args.summarize_only:
        summarize(args.out, args.report)
        return 0
    start = time.monotonic()
    deadline = start + args.max_hours * 3600
    args.binary, args.sizing_binary = args.binary.resolve(), args.sizing_binary.resolve()
    args.oop_range, args.ip_range = ROOT / 'data/ranges/bb_call.json', ROOT / 'data/ranges/btn_open.json'
    paths = [args.binary, args.sizing_binary, args.oop_range, args.ip_range]
    sources = sorted([*ROOT.glob('crates/**/*.rs'), *ROOT.glob('crates/**/Cargo.toml'), ROOT / 'Cargo.toml', ROOT / 'Cargo.lock', Path(__file__).resolve(), ROOT / 'tools/m3_bench.py'])
    config = dict(threads=args.threads, memory_mb=args.memory_mb, max_iters=args.max_iters,
                  targets=[args.stage1_target, args.stage2_target], **game_config())
    provenance = dict(hashes={str(p): m3.digest(p) for p in paths + sources}, config=config)
    identity = fingerprint(provenance)
    args.out.mkdir(parents=True, exist_ok=True)
    manifest_path = args.out / 'manifest.json'
    if manifest_path.exists():
        manifest = json.loads(manifest_path.read_text())
        if manifest['identity'] != identity:
            parser.error('resume provenance mismatch; use a new --out directory')
    else:
        directory = args.out / 'selection'
        directory.mkdir(exist_ok=True)
        command = [str(args.sizing_binary), 'select', '--training', str(args.training), '--validation', str(args.validation), '--seed', '20261015']
        resource = m3.run_child(command, directory, max(0.001, deadline - time.monotonic()))
        if resource['exit_code'] != 0:
            raise ValueError('selection subprocess failed')
        selection = json.loads((directory / 'stdout.log').read_text())
        if len(selection['training']) != args.training or len({r['flop'] for r in selection['training']}) != args.training:
            raise ValueError('invalid selection')
        manifest = dict(identity=identity, provenance=provenance, selection=selection,
                        selection_command=command, selection_resource=resource,
                        created_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                        budget_deadline_unix=time.time() + max(0, deadline - time.monotonic()))
        save(manifest_path, manifest)
    if args.prepare_only:
        print(f"prepared {len(manifest['selection']['training'])} training and {len(manifest['selection'].get('validation', []))} validation boards", flush=True)
        return 0
    # One persisted wall-clock window, including pauses between resumptions.
    # This deliberately errs conservative rather than resetting an eight-hour budget.
    deadline = min(deadline, time.monotonic() + max(0, manifest['budget_deadline_unix'] - time.time()))
    boards = manifest['selection']['training'][:args.limit] + manifest['selection'].get('validation', [])
    # Shard i/n takes every n-th board so parallel workers on one machine never share a board.
    boards = [b for i, b in enumerate(boards) if i % args.shard_count == args.shard_index]
    for selected in boards:
        board_path = args.out / 'flops' / (selected['flop'] + '.json')
        if board_path.exists():
            cached_board = json.loads(board_path.read_text())
            if cached_board.get('complete') and cached_board.get('identity') == identity and all(reusable(r, identity) for r in cached_board['stage1'] + cached_board['stage2']):
                print(selected['flop'] + ': complete (validated resume)', flush=True)
                continue
        if time.monotonic() >= deadline:
            break
        board = selected['flop']
        stage1 = []
        for size in SIZES:
            if time.monotonic() >= deadline:
                break
            stage1.append(candidate(args, board, size, 1, deadline, identity))
            save(args.out / 'flops' / f'{board}.json', board_result(selected, stage1, [], identity))
        stage2 = []
        if len(stage1) == 4 and all(r['status'] == 'complete' for r in stage1):
            top = sorted(stage1, key=lambda r: (-r['btn_ev_bb'], r['size']))[:2]
            for row in top:
                if time.monotonic() >= deadline:
                    break
                stage2.append(candidate(args, board, row['size'], 2, deadline, identity))
                save(args.out / 'flops' / f'{board}.json', board_result(selected, stage1, stage2, identity))
    # Parallel shards share one output directory; run --summarize-only once after all finish.
    if args.shard_count == 1:
        summarize(args.out, args.report)
    return 0


if __name__ == '__main__':
    sys.exit(main())
