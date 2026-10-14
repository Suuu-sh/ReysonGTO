#!/usr/bin/env python3
"""Read-only M2c diagnostic analyzer; writes only explicitly selected reports.

Input defaults: data/m2c/threads{4,6,8,10}. Missing optional evidence stays null.
No solver/build/OS commands, external dependencies, or inference of CPU type.
"""
import argparse
import json
from pathlib import Path
import re
import statistics


def read(path):
    if not path.exists():
        return None
    return json.loads(path.read_text())


def divide(a, b):
    return a / b if a is not None and b else None


def stats(values):
    return dict(count=len(values), min=min(values), p50=statistics.median(values),
                max=max(values), sum=sum(values)) if values else dict(count=0, min=None, p50=None, max=None, sum=None)


def os_delta(path):
    if not path.exists():
        return dict(available=False)
    rows = []
    errors = []
    for index, line in enumerate(path.read_text().splitlines()):
        try:
            row = json.loads(line)
            vm = row.get('vm_stat', {})
            raw = vm.get('stdout', '')
            counters = {name: int(value) for name, value in re.findall(r'^([^:\n]+):\s*(\d+)\.', raw, re.M)}
            size = re.search(r'page size of (\d+) bytes', raw)
            if counters:
                rows.append((row, counters, int(size.group(1)) if size else None))
            else:
                errors.append(dict(line=index + 1, vm_stat=vm))
        except (ValueError, TypeError) as error:
            errors.append(dict(line=index + 1, error=str(error)))
    if len(rows) < 2:
        return dict(available=False, valid_samples=len(rows), errors=errors)
    first, last = rows[0], rows[-1]
    names = ['Compressions', 'Decompressions', 'Pageouts', 'Swapins', 'Swapouts',
             'Pages stored in compressor', 'Pages occupied by compressor']
    counters = {name: dict(start=first[1].get(name), end=last[1].get(name),
                           delta=last[1][name] - first[1][name] if name in first[1] and name in last[1] else None)
                for name in names}
    return dict(available=True, valid_samples=len(rows), first_phase=first[0].get('phase'),
                last_phase=last[0].get('phase'), page_bytes=first[2], counters=counters,
                swap_start=first[0].get('swap'), swap_end=last[0].get('swap'), errors=errors,
                caveat='Host-wide counters; compression/decompression count repeated page processing, not resident bytes or solver-specific activity.')


def analyze(directory, threads):
    solution = read(directory / 'solution.json')
    resource = read(directory / 'resource.json')
    result = dict(directory=str(directory), threads=threads, resource=resource,
                  telemetry=os_delta(directory / 'telemetry.jsonl'))
    if solution is None:
        return result | dict(available=False, missing='solution.json')
    kd = solution.get('kernel_diagnostics')
    if not kd:
        return result | dict(available=False, missing='kernel_diagnostics')
    workload = kd.get('per_iteration_workload', {})
    iterations = kd.get('iterations', [])
    n = len(iterations)
    seconds = [i['times']['wall_ns'] / 1e9 for i in iterations if 'wall_ns' in i.get('times', {})]
    training = kd.get('training', {})
    batches = [b for i in iterations for b in i.get('chance_batches', [])]
    sum_batch_wall = sum(b['wall_ns'] for b in batches) if batches and all('wall_ns' in b for b in batches) else None
    finish_idle = sum(b['participating_worker_finish_idle_ns'] for b in batches) if batches and all('participating_worker_finish_idle_ns' in b for b in batches) else None
    tails = [b['final_task_tail_ns'] for b in batches if 'final_task_tail_ns' in b]
    tasks = kd.get('latest_training_tasks', [])
    groups = {}
    workers = {}
    for task in tasks:
        if not all(k in task for k in ('worker', 'start_ns', 'end_ns')):
            continue
        duration = task['end_ns'] - task['start_ns']
        workers.setdefault(str(task['worker']), []).append(duration)
        groups.setdefault((task.get('boundary_node'), task.get('target')), []).append(task)
    last_tasks = [max(group, key=lambda t: t['end_ns']) for group in groups.values()]
    last_durations = [t['end_ns'] - t['start_ns'] for t in last_tasks]
    parts = {}
    denominators = {'fold': workload.get('fold_dense_combos'),
                    'showdown': workload.get('showdown_dense_combos'),
                    'decision': workload.get('action_dense_combos'),
                    'decision_transport': workload.get('value_action_dense_combos'),
                    'chance': (workload['chance_reach_dense_combos'] + workload['chance_add_dense_combos'])
                    if all(k in workload for k in ('chance_reach_dense_combos', 'chance_add_dense_combos')) else None}
    for part, count in denominators.items():
        total_ns = training.get(part + '_ns')
        parts[part] = dict(training_worker_ns=total_ns, dense_combos_per_iteration=count,
                           ns_per_dense_combo=divide(total_ns, n * count if count else None))
    fold_ops = (3 * workload['fold_dense_combos'] + 4 * workload['fold_active_combos']) if all(k in workload for k in ('fold_dense_combos', 'fold_active_combos')) else None
    showdown_ops = (12 * workload['showdown_dense_combos'] + 3 * workload['showdown_prefix_combos'] + 4 * workload['showdown_active_combos']) if all(k in workload for k in ('showdown_dense_combos', 'showdown_prefix_combos', 'showdown_active_combos')) else None
    arithmetic = {part: dict(arithmetic_lower_bound_ops_per_iteration=ops,
                            worker_ns_per_proxy_op=divide(training.get(part + '_ns'), n * ops if ops else None))
                  for part, ops in [('fold', fold_ops), ('showdown', showdown_ops)]}
    return result | dict(available=True, iterations=solution.get('iterations'), diagnostic_iterations=n,
        training_seconds=solution.get('training_seconds'), metrics_seconds=solution.get('metrics_seconds'),
        training_diagnostic_seconds=divide(training.get('wall_ns'), 1e9),
        training_seconds_per_iteration=divide(training.get('wall_ns'), n * 1e9),
        iteration_seconds=stats(seconds), exploitability_pct=(solution.get('metrics') or {}).get('exploitability_pct'),
        per_iteration_workload=workload,
        physical_nodes_per_iteration={k: divide(workload.get(k), 2) for k in ('decision_visits', 'fold_visits', 'showdown_visits', 'chance_visits')},
        chance=dict(batches=len(batches), tail_samples=len(tails), summed_batch_wall_seconds=divide(sum_batch_wall, 1e9),
                    final_task_tail_seconds=stats([v / 1e9 for v in tails]),
                    participating_worker_finish_idle_seconds=divide(finish_idle, 1e9),
                    finish_idle_capacity_proxy=divide(finish_idle, threads * sum_batch_wall) if sum_batch_wall else None,
                    task_min_seconds=min((b['task_min_ns'] / 1e9 for b in batches if 'task_min_ns' in b), default=None),
                    task_max_seconds=max((b['task_max_ns'] / 1e9 for b in batches if 'task_max_ns' in b), default=None),
                    latest_batch_last_finishing_task_seconds=stats([v / 1e9 for v in last_durations]),
                    latest_training_worker_identities={worker: stats([v / 1e9 for v in values]) for worker, values in sorted(workers.items(), key=lambda item: int(item[0]))}),
        parts=parts, primitive_arithmetic_lower_bound_proxy=arithmetic,
        metrics_diagnostics={k: kd.get(k) for k in ('average_evaluation', 'best_response_evaluation')})


def fmt(value, digits=3):
    return 'missing' if value is None else f'{value:.{digits}f}'


def markdown(report):
    lines = ['# M2c fixed-input diagnostic comparison', '',
             'Single sequential samples; training only is separated from exact metrics. No target-accuracy speed claim.', '',
             '| threads | training s/iter | p50 / min / max s | max RSS GB | tails sum / max s | last-finishing task max s (latest iteration) | finish-idle proxy % | task min / max s | metrics s |',
             '| ---: | ---: | --- | ---: | --- | ---: | ---: | --- | ---: |']
    for r in report['runs']:
        if not r.get('available'):
            lines.append(f"| {r['threads']} | missing | | | | | | | |")
            continue
        s, c = r['iteration_seconds'], r['chance']
        rss = (r.get('resource') or {}).get('maximum_resident_set_bytes')
        lines.append(f"| {r['threads']} | {fmt(r['training_seconds_per_iteration'])} | {fmt(s['p50'])} / {fmt(s['min'])} / {fmt(s['max'])} | {fmt(divide(rss, 1e9))} | {fmt(c['final_task_tail_seconds']['sum'])} / {fmt(c['final_task_tail_seconds']['max'])} | {fmt(c['latest_batch_last_finishing_task_seconds']['max'])} | {fmt(divide(c['finish_idle_capacity_proxy'], .01))} | {fmt(c['task_min_seconds'])} / {fmt(c['task_max_seconds'])} | {fmt(r['metrics_seconds'])} |")
    lines += ['', 'Finish-idle proxy = summed participating-worker finish-idle / (configured threads × summed batch wall). It excludes initial/inter-task idle, nonparticipants and non-chance work: **not full idle/utilization**. Tail values are batch tails, not additive solver wall cost. Worker IDs are Rayon identities, **not P/E CPU types**.', '',
              '## Training worker nanoseconds per counted dense combo', '',
              '| threads | fold/output | showdown/output | decision/action | transport/value-action | chance/(reach+add) | fold ns/proxy op | showdown ns/proxy op |', '| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |']
    for r in report['runs']:
        if not r.get('available'):
            continue
        values = [r['parts'][p]['ns_per_dense_combo'] for p in ('fold', 'showdown', 'decision', 'decision_transport', 'chance')]
        values += [r['primitive_arithmetic_lower_bound_proxy'][p]['worker_ns_per_proxy_op'] for p in ('fold', 'showdown')]
        lines.append('| ' + str(r['threads']) + ' | ' + ' | '.join(fmt(v) for v in values) + ' |')
    lines += ['', 'These are summed worker timer ns / workload, **not wall ns or actual instruction timings**. Decision includes normalization/update; transport is separate. Counts are exact traversal work counters, but timer scopes also include loads/stores, branches and timer costs.', '',
              'Arithmetic lower-bound proxy per iteration: fold = 3×fold_dense + 4×fold_active; showdown = 3×showdown_dense (totals) + 3×prefix + 9×showdown_dense (three compatible calculations) + 4×showdown_active (mixing). Excludes max/comparisons/conversions/timer overhead; no FLOP/s or bandwidth conclusion.', '', '## Workload / physical nodes', '']
    valid = [r for r in report['runs'] if r.get('available')]
    if valid:
        lines += ['Counts below are per iteration (two target traversals). Physical-node counts = visit counts / 2.', '', '| category | visits | physical nodes |', '| --- | ---: | ---: |']
        for k, v in valid[0]['physical_nodes_per_iteration'].items():
            lines.append(f"| {k} | {valid[0]['per_iteration_workload'].get(k)} | {fmt(v, 0)} |")
        lines += ['', 'Final iterations / exploitability (initial pot %): ' + '; '.join(f"threads {r['threads']}: {r['iterations']} / {fmt(r['exploitability_pct'], 9)}%" for r in valid) + '.']
        equal = all(r['per_iteration_workload'] == valid[0]['per_iteration_workload'] for r in valid)
        lines += ['', 'Workload identical across available runs: ' + str(equal) + '. Full dense/active counts retained in JSON.']
    lines += ['', '## Latest-iteration worker identities', '']
    for r in valid:
        lines.append(f"- threads {r['threads']}: " + '; '.join(f"worker {k}: {v['count']} tasks, summed {fmt(v['sum'])} s, min/max {fmt(v['min'])}/{fmt(v['max'])} s" for k, v in r['chance']['latest_training_worker_identities'].items()))
    lines += ['', '## Host OS deltas (raw vm_stat)', '', '| threads | Compressions | Decompressions | Pageouts | Swapins | Swapouts |', '| ---: | ---: | ---: | ---: | ---: | ---: |']
    for r in report['runs']:
        counters = r['telemetry'].get('counters', {})
        lines.append('| ' + str(r['threads']) + ' | ' + ' | '.join(str(counters.get(k, {}).get('delta', 'missing')) for k in ('Compressions', 'Decompressions', 'Pageouts', 'Swapins', 'Swapouts')) + ' |')
    lines += ['', 'Host-wide cumulative counters, not solver-specific. Compression/decompression can process the same pages repeatedly; page counts are not resident capacity. Raw swap start/end, phase coverage and page size are preserved in JSON. No thermal/CPU-type/root-cause inference.', '',
              'Missing optional evidence is represented as null/missing. Metrics work is separately preserved; none is silently added to training denominators.']
    return '\n'.join(lines) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path('data/m2c'))
    parser.add_argument('--out', type=Path, default=Path('data/m2c/diagnostic-analysis'), help='report basename (.json and .md appended)')
    args = parser.parse_args()
    report = dict(runs=[analyze(args.root / f'threads{t}', t) for t in (4, 6, 8, 10)],
                  scope='Read-only sequential fixed-input diagnostics; no full idle or actual instruction timing claim.')
    args.out.parent.mkdir(parents=True, exist_ok=True)
    Path(str(args.out) + '.json').write_text(json.dumps(report, indent=2) + '\n')
    Path(str(args.out) + '.md').write_text(markdown(report))


if __name__ == '__main__':
    main()
