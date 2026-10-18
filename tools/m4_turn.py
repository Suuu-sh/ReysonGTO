#!/usr/bin/env python3
"""Resumable serial turn sizing comparison; no builds or production range edits."""
import argparse
import datetime
import json
import math
from pathlib import Path
import sys
import time

import m3_bench as m3
from m4_trial import save, fingerprint

ROOT = Path(__file__).resolve().parents[1]
SIZES = (33, 66, 100, 125)
LINES = {'XX': 'X,X', 'BC': 'X,B,C'}


def candidate_rules(size, bettor):
    return dict(version=1, candidates=[s / 100 for s in SIZES], default_fraction=.66,
                rules=[dict(street='Turn', player=bettor, after_check=bettor == 1,
                            fraction=size / 100, sources=['m4-turn temporary candidate'])])


def overlay_candidate(table, size, bettor, street, context):
    """Override only this decision, including more-specific supplied rows.

    Copies precede originals so equal-specificity ties also select the candidate;
    original rules remain intact for every non-compared decision.
    """
    scope = dict(context, street=street, player=bettor, after_check=bettor == 1)
    overrides = []
    for row in table['rules']:
        constraints = {k: v for k, v in row.items()
                       if k not in ('fraction', 'sources') and v is not None}
        if all(k not in scope or scope[k] == v for k, v in constraints.items()):
            overrides.append(dict(constraints, **{k: v for k, v in scope.items() if k not in constraints},
                                  fraction=size / 100, sources=['temporary compared-context candidate']))
    overrides.append(dict(scope, fraction=size / 100, sources=['temporary compared-context candidate']))
    return dict(table, rules=overrides + table['rules'])


def sizing_policy(args):
    path = getattr(args, 'other_side_rules', None)
    if path is None:
        return dict(mode='legacy_default_66', default_fraction=.66)
    return dict(mode='supplied_full_table', path=str(path), sha256=m3.digest(path))


def supplied_table(args):
    path = getattr(args, 'other_side_rules', None)
    return json.loads(path.read_text()) if path is not None else None


def comparison_rules(args, size, spot, street):
    table = supplied_table(args)
    if table is None:
        rules = candidate_rules(size, spot['bettor'])
        rules['rules'][0].update(street=street, sources=['m4-'+street.lower()+' temporary candidate'])
        return rules
    line = spot['line'] if street == 'Turn' else spot['turn_line']
    context = dict(previous_street_line={'XX': 'checked_through', 'BC': 'ip_bet_called'}[line],
                   runout=spot['kind'] if street == 'Turn' else spot['river_kind'])
    return overlay_candidate(table, size, spot['bettor'], street, context)


def flop_rules(table, features):
    """Materialize exact original matching semantics for this fixed flop."""
    rules = []
    for player in (0, 1):
        for after_check in (False, True):
            context = dict(features, street='Flop', player=player, after_check=after_check, runout=None)
            best, fraction = -1, table['default_fraction']
            for row in table['rules']:
                constraints = {k: v for k, v in row.items() if k not in ('fraction', 'sources') and v is not None}
                if all(context.get(k) == v for k, v in constraints.items()) and len(constraints) > best:
                    best, fraction = len(constraints), row['fraction']
            rules.append(dict(street='Flop', player=player, after_check=after_check,
                              fraction=fraction, sources=['materialized supplied flop rules']))
    return dict(version=table['version'], candidates=table['candidates'], default_fraction=.66, rules=rules)


def select_flops(value, manifest_path):
    if value.isdigit():
        count = int(value)
        rows = json.loads(manifest_path.read_text())['selection']['training']
        if not 0 < count <= len(rows):
            raise ValueError('--flops count exceeds training selection or is zero')
        return rows[:count]
    boards = value.split(',')
    for board in boards:
        cards = [board[i:i+2] for i in range(0, len(board), 2)]
        if len(board) != 6 or len(set(cards)) != 3 or any(c[0] not in '23456789TJQKA' or c[1] not in 'cdhs' for c in cards):
            raise ValueError('invalid explicit flop: ' + board)
    if len(set(boards)) != len(boards):
        raise ValueError('duplicate flops')
    return [dict(flop=b) for b in boards]


def reusable(record, identity):
    if record.get('status') != 'complete' or record.get('identity') != identity:
        return False
    try:
        return all(m3.digest(Path(p)) == digest for p, digest in record['artifacts'].items())
    except (OSError, KeyError):
        return False


def run_attempt(directory, identity, deadline, build, validate):
    directory.mkdir(parents=True, exist_ok=True)
    cached = directory / 'result.json'
    if cached.exists():
        previous = json.loads(cached.read_text())
        if reusable(previous, identity):
            return previous
    if time.monotonic() >= deadline:
        return dict(status='budget_exhausted', identity=identity)
    n = 1
    while (directory / f'attempt-{n}').exists():
        n += 1
    attempt = directory / f'attempt-{n}'
    attempt.mkdir()
    record = dict(identity=identity, attempt=str(attempt))
    try:
        command, artifacts = build(attempt)
        resource = m3.run_child(command, attempt, max(.001, deadline - time.monotonic()))
        checkpoints, errors = m3.checkpoints(attempt / 'progress.jsonl')
        record.update(resource=resource, checkpoints=checkpoints, progress_parse_errors=errors)
        if resource['timed_out'] or resource['exit_code'] != 0:
            raise ValueError('timeout or process failure; see retained stderr.log')
        record.update(validate(attempt))
        record.update(status='complete' if record.get('target_met', True) else 'target_not_met',
                      artifacts={str(p): m3.digest(p) for p in artifacts})
    except (OSError, ValueError, KeyError, TypeError) as error:
        record.update(status='invalid_or_incomplete', error=str(error))
    save(attempt / 'result.json', record)
    save(cached, record)
    return record


def chips(bb):
    """BB to chips exactly as the CLI's chips_at: f64 multiply, round half away from zero."""
    return math.floor(bb * 1000 + 0.5)


def validate_solution(directory, board, args, pot):
    report = json.loads((directory/'solution.json').read_text())
    spec = report['game_spec']
    expected = ['23456789TJQKA'.index(board[i])*4+'cdhs'.index(board[i+1]) for i in range(0, len(board), 2)]
    if spec['board'] != expected or spec['units_per_bb'] != 1000 or spec['tree']['starting_pot'] != pot:
        raise ValueError('solution board, units or pot differs from request')
    if spec.get('size_rules') != json.loads((directory/'rules.json').read_text()):
        # Rust serde fills optional rule fields with null; compare normalized forms.
        normalize = lambda table: dict(table, rules=[{k: v for k, v in row.items() if v is not None} for row in table['rules']])
        if not spec.get('size_rules') or normalize(spec['size_rules']) != normalize(json.loads((directory/'rules.json').read_text())):
            raise ValueError('solution size rules differ from request')
    if spec['engine']['threads'] != args.threads or report['root_ev_chips'] != report['metrics']['average_ev']:
        raise ValueError('solution engine or root EV mismatch')


def read_metrics(path, target, bettor):
    report = json.loads(path.read_text())
    raw = report['metrics']
    units = report['game_spec']['units_per_bb']
    pot = report['game_spec']['tree']['starting_pot']
    ev, br, exploit = raw['average_ev'], raw['best_response_ev'], raw['exploitability_pct']
    values = [*ev, *br, exploit, units, pot]
    if len(ev) != 2 or len(br) != 2 or any(isinstance(x, bool) or not isinstance(x, (int, float)) or not math.isfinite(x) for x in values):
        raise ValueError('invalid exact metrics')
    if min(units, pot) <= 0 or exploit < 0 or min(br[p]-ev[p] for p in range(2)) < -1e-4:
        raise ValueError('invalid metric units or BR improvement')
    if abs(sum(br[p]-ev[p] for p in range(2)) / 2 / pot * 100-exploit) > 1e-5:
        raise ValueError('inconsistent exploitability')
    return dict(bettor_ev_bb=ev[bettor]/units, average_ev_bb=[x/units for x in ev],
                exploitability_pct=exploit, target_met=exploit <= target,
                diagnostic_proxy_bb=2*exploit/100*pot/units,
                exact_metrics=raw, iterations=report['iterations'], solve_seconds=report['solve_seconds'],
                stopped_by=report['stopped_by'])


def engine_args(args, target, directory):
    return ['--threads', str(args.threads), '--memory-mb', str(args.memory_mb),
            '--storage', 'f32', '--algorithm', 'dcfr', '--dcfr-alpha', '1.5',
            '--dcfr-beta', '0', '--dcfr-gamma', '2', '--target', str(target),
            '--max-iters', str(args.max_iters), '--max-seconds', str(args.max_solve_seconds),
            '--progress-jsonl', str(directory/'progress.jsonl'),
            '--out', str(directory/'solution.json')]


def solve_flop(args, selected, cards, identity, deadline):
    board = selected['flop']
    def build(d):
        requests = [dict(line=LINES[line], card=card, out=str(d/f'{line}-{card}.json'))
                    for line in args.lines for group in cards['groups'] for card in group['cards']]
        save(d/'reach-requests.json', requests)
        table = supplied_table(args)
        save(d/'rules.json', table if table is not None else flop_rules(json.loads(args.flop_rule.read_text()), cards['features']))
        command = [str(args.binary), 'solve', '--preset', 'production', '--board', board,
                   '--oop-range', str(args.oop_range), '--ip-range', str(args.ip_range),
                   '--pot', str(args.pot), '--stack', str(args.stack), '--units-per-bb', '1000',
                   '--no-oop-leads', '--size-rules', str(d/'rules.json'),
                   '--reach-requests', str(d/'reach-requests.json')] + engine_args(args, args.flop_target, d)
        return command, [d/'solution.json', d/'rules.json', d/'reach-requests.json']+[Path(r['out']) for r in requests]
    def validate(d):
        validate_solution(d, board, args, chips(args.pot))
        metrics = read_metrics(d/'solution.json', args.flop_target, 1)
        if not metrics['target_met']:
            raise ValueError('flop target not met; refusing inaccurate reach source')
        return dict(metrics=metrics, reach_directory=str(d))
    return run_attempt(args.out/'flops'/board, identity, deadline, build, validate)


def candidate(args, spot, state, stage, size, identity, deadline):
    target = args.coarse_target if stage == 'coarse' else args.fine_target
    policy = sizing_policy(args)
    local_identity = fingerprint([identity, m3.digest(state), spot, stage, size, policy])
    def build(d):
        save(d/'rules.json', comparison_rules(args, size, spot, 'Turn'))
        command = [str(args.binary), 'solve-subgame', '--state', str(state),
                   '--size-rules', str(d/'rules.json')] + engine_args(args, target, d)
        return command, [d/'solution.json', d/'rules.json']
    def validate(d):
        snapshot = json.loads(state.read_text())
        validate_solution(d, spot['flop'] + spot['card'], args, snapshot['pot'])
        return read_metrics(d/'solution.json', target, spot['bettor'])
    result = run_attempt(args.out/'spots'/spot['id']/f'{stage}-{size}', local_identity, deadline,
                         build, validate)
    return dict(result, size=size, stage=stage, target_pct=target, non_compared_sizing_policy=policy)


def compare(args, spot, state, identity, deadline):
    coarse = [candidate(args, spot, state, 'coarse', size, identity, deadline) for size in SIZES]
    fine = []
    if all(r['status'] == 'complete' and r['target_met'] for r in coarse):
        top = sorted(coarse, key=lambda r: (-r['bettor_ev_bb'], r['size']))[:2]
        fine = [candidate(args, spot, state, 'fine', r['size'], identity, deadline) for r in top]
    complete = len(fine) == 2 and all(r['status'] == 'complete' and r['target_met'] for r in fine)
    refined = sorted(fine, key=lambda r: (-r.get('bettor_ev_bb', -math.inf), r['size']))
    result = dict(**spot, coarse=coarse, fine=fine, complete=complete,
                  observed_winner=refined[0]['size'] if complete else None, no_optimality_claim=True)
    if complete:
        gap = refined[0]['bettor_ev_bb']-refined[1]['bettor_ev_bb']
        proxy = sum(r['diagnostic_proxy_bb'] for r in refined)
        result.update(gap_bb=gap, combined_proxy_bb=proxy, indistinguishable_by_proxy=gap <= proxy)
    save(args.out/'spots'/spot['id']/'summary.json', result)
    return result


def summarize(args, manifest, spots, failures):
    winner_counts = {}
    for spot in spots:
        if spot['complete']:
            group = f"{spot['kind']}/p{spot['bettor']}"
            size = str(spot['observed_winner'])
            counts = winner_counts.setdefault(group, {})
            counts[size] = counts.get(size, 0) + 1
    summary = dict(non_compared_sizing_policy=sizing_policy(args), identity=manifest['identity'], spots=spots, failures=failures, winner_counts=winner_counts,
                   completed=sum(r['complete'] for r in spots), total=len(spots), no_optimality_claim=True)
    save(args.out/'summary.json', summary)
    lines = ['# M4 turn sizing comparison', '',
             'Non-compared sizing policy: '+json.dumps(summary['non_compared_sizing_policy'], sort_keys=True), '',
             'Bettor root EV (OOP=0 lead; IP=1 after check), not conditional bet action EV.',
             'Coarse four candidates; fine top two only. Eliminated sizes remain unrefined.',
             'No optimality claim: general-sum raked exploitability is not an equilibrium EV bound.',
             'Gap <= combined heuristic proxy is labelled indistinguishable. No held-out validation.', '',
             '| Flop / features | Line | Card / kind | Bettor | Stage | Size | EV BB | Exploit % | Wall sec / RSS bytes |',
             '|---|---|---|---:|---|---:|---:|---:|---|']
    for spot in spots:
        for r in spot['coarse']+spot['fine']:
            resource = r.get('resource', {})
            lines.append(f"| {spot['flop']} {json.dumps(spot['features'], sort_keys=True)} | {spot['line']} | {spot['card']} / {spot['kind']} | {spot['bettor']} | {r['stage']} | {r['size']} | {r.get('bettor_ev_bb', r['status'])} | {r.get('exploitability_pct', '—')} | {resource.get('wall_seconds', '—')} / {resource.get('maximum_resident_set_bytes', '—')} |")
    lines += ['', '## Counts and observed winners by kind / bettor',
              f"Completed situations: {summary['completed']} / {summary['total']}; failed source flops: {len(failures)}",
              'Descriptive refined-survivor counts only; OOP and IP remain separate.',
              '```json', json.dumps(winner_counts, indent=2), '```', '', '## Refined survivors']
    for spot in spots:
        lines.append(f"- {spot['id']}: observed winner={spot['observed_winner']}; complete={spot['complete']}; gap BB={spot.get('gap_bb')}; combined proxy BB={spot.get('combined_proxy_bb')}; indistinguishable={spot.get('indistinguishable_by_proxy')}")
    lines += ['', '## Failures', json.dumps(failures, indent=2), '', '## Provenance',
              'Immutable manifest includes supplied rule/input/source/binary hashes; failed attempts retained.',
              '```json', json.dumps(manifest, indent=2), '```']
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text('\n'.join(lines)+'\n')
    return summary


def main(argv=None):
    global SIZES
    parser = argparse.ArgumentParser(description=__doc__)
    for name, default in [('binary', ROOT/'target/release/cli'), ('sizing-binary', ROOT/'target/release/sizing'),
                          ('out', ROOT/'data/m4-turn'), ('report', ROOT/'docs/benchmarks/m4-turn.md'),
                          ('flop-rule', ROOT/'configs/sizing/srp-btn-vs-bb.json'),
                          ('manifest', ROOT/'data/m4b/manifest.json'), ('oop-range', ROOT/'data/ranges/bb_call.json'),
                          ('ip-range', ROOT/'data/ranges/btn_open.json')]:
        parser.add_argument('--'+name, type=Path, default=default)
    parser.add_argument('--other-side-rules', type=Path, help='full rules for all non-compared decisions')
    parser.add_argument('--flops', default='20', help='first N training flops, or comma separated boards')
    parser.add_argument('--lines', default='XX,BC')
    parser.add_argument('--sizes', default=','.join(map(str, SIZES)), help='candidate bet sizes in percent of the pot')
    for name, default in [('threads', 4), ('memory-mb', 16384), ('max-iters', 10000), ('per-kind', 1), ('seed', 20261015)]:
        parser.add_argument('--'+name, type=int, default=default)
    for name, default in [('coarse-target', 5.), ('fine-target', .5), ('flop-target', 1.), ('max-hours', 8.), ('max-solve-seconds', 7200.), ('pot', 5.5), ('stack', 97.5)]:
        parser.add_argument('--'+name, type=float, default=default)
    args = parser.parse_args(argv)
    args.lines = args.lines.split(',')
    # Candidates are part of the run's identity through vars(args).
    SIZES = tuple(int(x) for x in args.sizes.split(','))
    if len(SIZES) < 2 or len(set(SIZES)) != len(SIZES) or min(SIZES) <= 0:
        parser.error('--sizes needs at least two distinct positive sizes')
    if not args.lines or len(set(args.lines)) != len(args.lines) or any(x not in LINES for x in args.lines):
        parser.error('--lines must be XX, BC, or XX,BC')
    if min(args.threads, args.memory_mb, args.max_iters, args.per_kind) <= 0 or any(not math.isfinite(x) or x <= 0 for x in [args.coarse_target, args.fine_target, args.flop_target, args.max_hours, args.max_solve_seconds, args.pot, args.stack]) or args.fine_target > args.coarse_target:
        parser.error('positive finite resources and fine-target <= coarse-target required')
    for key, value in vars(args).items():
        if isinstance(value, Path):
            setattr(args, key, value.resolve())
    if args.other_side_rules is None and not args.flop_rule.is_file():
        parser.error('supplied flop rule file missing; no inferred or fabricated rules are used: '+str(args.flop_rule))
    try:
        selection = select_flops(args.flops, args.manifest)
        paths = [args.binary, args.sizing_binary, args.other_side_rules or args.flop_rule, args.oop_range, args.ip_range,
                 Path(__file__).resolve(), ROOT/'tools/m3_bench.py', ROOT/'tools/m4_trial.py']
        if args.flops.isdigit():
            paths.append(args.manifest)
        paths += sorted(ROOT.glob('crates/**/*.rs')) + sorted(ROOT.glob('crates/**/Cargo.toml')) + [ROOT/'Cargo.lock', ROOT/'Cargo.toml']
        config = {k: str(v) if isinstance(v, Path) else v for k, v in vars(args).items() if k not in ('out', 'report', 'max_hours', 'max_solve_seconds')}
        provenance = dict(non_compared_sizing_policy=sizing_policy(args), hashes={str(p): m3.digest(p) for p in paths}, config=config, selection=selection)
    except (OSError, ValueError, KeyError) as e:
        parser.error(str(e))
    identity = fingerprint(provenance)
    args.out.mkdir(parents=True, exist_ok=True)
    path = args.out/'manifest.json'
    if path.exists():
        manifest = json.loads(path.read_text())
        if manifest['identity'] != identity:
            parser.error('resume provenance mismatch; use a new --out directory')
    else:
        manifest = dict(identity=identity, provenance=provenance,
                        created_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                        budget_deadline_unix=time.time()+args.max_hours*3600)
        save(path, manifest)
    deadline = time.monotonic()+max(0, min(args.max_hours*3600, manifest['budget_deadline_unix']-time.time()))
    spots, failures = [], []
    for selected in selection:
        board = selected['flop']
        def build(d):
            return [str(args.sizing_binary), 'turn-cards', '--flop', board, '--per-kind', str(args.per_kind), '--seed', str(args.seed)], [d/'stdout.log']
        selection_record = run_attempt(args.out/'cards'/board, identity, deadline, build,
                                      lambda d: dict(cards=json.loads((d/'stdout.log').read_text())))
        if selection_record['status'] != 'complete':
            failures.append(dict(flop=board, phase='cards', record=selection_record)); continue
        cards = selection_record['cards']
        flop = solve_flop(args, selected, cards, identity, deadline)
        if flop['status'] != 'complete':
            failures.append(dict(flop=board, phase='flop', record=flop)); continue
        for line in args.lines:
            for group in cards['groups']:
                for card in group['cards']:
                    state = Path(flop['reach_directory'])/f'{line}-{card}.json'
                    for bettor in (0, 1):
                        spot = dict(id=f'{board}-{line}-{card}-p{bettor}', flop=board,
                                    features=selected.get('features', cards.get('features', {})), line=line,
                                    kind=group['kind'], card=card, bettor=bettor, after_check=bettor == 1)
                        spots.append(compare(args, spot, state, identity, deadline))
    summary = summarize(args, manifest, spots, failures)
    print(json.dumps(dict(completed=summary['completed'], total=summary['total'], failures=len(failures), report=str(args.report))))
    return 0 if not failures and spots and all(s['complete'] for s in spots) else 1


if __name__ == '__main__':
    sys.exit(main())
