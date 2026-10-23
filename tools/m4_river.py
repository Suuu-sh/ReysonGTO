#!/usr/bin/env python3
"""Resumable serial river sizing comparison; never builds or edits input ranges."""
import argparse
import datetime
import json
import math
from pathlib import Path
import sys
import time

import m4_turn as turn
from m4_trial import save, fingerprint

ROOT = turn.ROOT
SIZES = turn.SIZES
LINES = turn.LINES
TURN_KINDS = ('BoardPair', 'Overcard', 'Blank', 'FlushDraw', 'FlushComplete', 'StraightComplete')


def candidate_rules(size, bettor):
    table = turn.candidate_rules(size, bettor)
    table['rules'][0].update(street='River', sources=['m4-river temporary candidate'])
    return table


def source_rules(table, features=None):
    """Materialize original first-tie-wins semantics for one fixed flop.

    Keep every possible turn kind/history: source flop solve can visit all
    turns, not just the representatives later exported for river comparison.
    """
    rules = []
    for street in ('Flop', 'Turn'):
        for player in (0, 1):
            for after_check in (False, True):
                for previous in ((None,) if street == 'Flop' else ('checked_through', 'ip_bet_called', 'lead_called', 'raised')):
                    for runout in ((None,) if street == 'Flop' else TURN_KINDS):
                        context = dict(features or {}, street=street, player=player,
                                       after_check=after_check, previous_street_line=previous, runout=runout)
                        best, fraction = -1, table['default_fraction']
                        for row in table['rules']:
                            constraints = {k: v for k, v in row.items()
                                           if k not in ('fraction', 'sources') and v is not None}
                            if all(context.get(k) == v for k, v in constraints.items()) and len(constraints) > best:
                                best, fraction = len(constraints), row['fraction']
                        rules.append(dict(street=street, player=player, after_check=after_check,
                                          previous_street_line=previous, runout=runout, fraction=fraction,
                                          sources=['materialized supplied flop/turn rules']))
    return dict(version=table['version'], candidates=table['candidates'], default_fraction=.66, rules=rules)


def select_turns(cards, kinds, max_turns):
    """Kind argument order is selection priority; retain at most one per kind."""
    remaining, groups = max_turns, []
    for kind in kinds:
        matching = [g for g in cards['groups'] if g['kind'] == kind]
        chosen = matching[0]['cards'][:min(1, remaining)] if matching else []
        groups.append(dict(kind=kind, cards=chosen))
        remaining -= len(chosen)
    return dict(cards, groups=groups)


def estimate_counts(flop_count, turn_count=3, river_count=5):
    """Upper bounds per requested representative counts, not timing evidence."""
    turns = flop_count * 2 * turn_count
    situations = turns * 2 * river_count * 2
    return dict(flops=flop_count, turn_solves=turns, river_situations=situations,
                river_solves=situations * len(SIZES), total_solves=flop_count + turns + situations * len(SIZES))


def card_selection(args, board, street, identity, deadline):
    def build(d):
        command = [str(args.sizing_binary), street+'-cards', '--flop' if street == 'turn' else '--board',
                   board, '--per-kind', '1', '--seed', str(args.seed)]
        return command, [d/'stdout.log']
    def validate(d):
        cards = json.loads((d/'stdout.log').read_text())
        seen = set()
        for group in cards['groups']:
            if group['kind'] not in TURN_KINDS or len(group['cards']) > 1:
                raise ValueError('unexpected representative group')
            for card in group['cards']:
                if len(card) != 2 or card[0] not in '23456789TJQKA' or card[1] not in 'cdhs' or card in seen or card in [board[i:i+2] for i in range(0, len(board), 2)]:
                    raise ValueError('invalid or duplicate runout card')
                seen.add(card)
        return dict(cards=cards)
    return turn.run_attempt(args.out/'cards'/board, identity, deadline, build, validate)


def solve_source(args, board, cards, identity, deadline, state=None, source_id=None):
    street = 'flop' if state is None else 'turn'
    target = args.flop_target if state is None else args.turn_target
    local_identity = fingerprint([identity, board, cards, m3_digest(state) if state else None])
    def build(d):
        requests = [dict(line=LINES[line], card=card, out=str(d/f'{line}-{card}.json'))
                    for line in args.lines for group in cards['groups'] for card in group['cards']]
        table = turn.supplied_table(args)
        save(d/'rules.json', table if table is not None else source_rules(json.loads(args.flop_rule.read_text()), cards['features']))
        save(d/'reach-requests.json', requests)
        if state is None:
            command = [str(args.binary), 'solve', '--preset', 'production', '--board', board,
                       '--oop-range', str(args.oop_range), '--ip-range', str(args.ip_range),
                       '--pot', str(args.pot), '--stack', str(args.stack), '--units-per-bb', '1000', '--no-oop-leads']
        else:
            command = [str(args.binary), 'solve-subgame', '--state', str(state)]
        command += ['--size-rules', str(d/'rules.json'), '--reach-requests', str(d/'reach-requests.json')]
        return command + turn.engine_args(args, target, d), [d/'solution.json', d/'rules.json', d/'reach-requests.json'] + [Path(r['out']) for r in requests]
    def validate(d):
        pot = turn.chips(args.pot) if state is None else json.loads(state.read_text())['pot']
        turn.validate_solution(d, board, args, pot)
        metrics = turn.read_metrics(d/'solution.json', target, 1)
        for request in json.loads((d/'reach-requests.json').read_text()):
            snapshot = json.loads(Path(request['out']).read_text())
            expected_board = board + request['card']
            expected_ids = ['23456789TJQKA'.index(expected_board[i])*4+'cdhs'.index(expected_board[i+1]) for i in range(0, len(expected_board), 2)]
            if snapshot['board'] != expected_ids or snapshot['units_per_bb'] != 1000 or snapshot['pot'] <= 0 or not snapshot['is_street_start']:
                raise ValueError('invalid reach snapshot board, pot, units or street start')
        return dict(metrics=metrics, target_met=metrics['target_met'], reach_directory=str(d))
    return turn.run_attempt(args.out/(street+'s')/(source_id or board), local_identity, deadline, build, validate)


m3_digest = turn.m3.digest


def candidate(args, spot, state, size, identity, deadline):
    policy = turn.sizing_policy(args)
    local_identity = fingerprint([identity, m3_digest(state), spot, size, policy])
    def build(d):
        save(d/'rules.json', turn.comparison_rules(args, size, spot, 'River'))
        command = [str(args.binary), 'solve-subgame', '--state', str(state), '--size-rules', str(d/'rules.json')]
        return command + turn.engine_args(args, args.river_target, d), [d/'solution.json', d/'rules.json']
    def validate(d):
        turn.validate_solution(d, spot['flop']+spot['turn_card']+spot['river_card'], args, json.loads(state.read_text())['pot'])
        return turn.read_metrics(d/'solution.json', args.river_target, spot['bettor'])
    result = turn.run_attempt(args.out/'spots'/spot['id']/str(size), local_identity, deadline, build, validate)
    return dict(result, size=size, target_pct=args.river_target, non_compared_sizing_policy=policy)


def compare(args, spot, state, identity, deadline):
    candidates = [candidate(args, spot, state, size, identity, deadline) for size in SIZES]
    complete = all(r['status'] == 'complete' and r['target_met'] for r in candidates)
    winner = None
    if complete:
        best = sorted(candidates, key=lambda r: (-r['bettor_ev_bb'], r['size']))[0]
        winner = best['size']
        for row in candidates:
            row['loss_pct_pot'] = (best['bettor_ev_bb']-row['bettor_ev_bb'])/spot['river_pot_bb']*100
    result = dict(spot, candidates=candidates, complete=complete, observed_winner=winner, no_optimality_claim=True)
    save(args.out/'spots'/spot['id']/'summary.json', result)
    return result


def summarize(args, manifest, spots, failures, sources):
    counts = {}
    for spot in spots:
        if spot['complete']:
            key = spot['river_kind']+'/p'+str(spot['bettor'])
            bucket = counts.setdefault(key, {})
            size = str(spot['observed_winner'])
            bucket[size] = bucket.get(size, 0)+1
    timings = {}
    for phase in ('flop', 'turn', 'river'):
        records = [r['record'] for r in sources if r['phase'] == phase] if phase != 'river' else [r for s in spots for r in s['candidates']]
        walls = [r['resource']['wall_seconds'] for r in records if r.get('status') == 'complete' and 'resource' in r]
        timings[phase] = dict(completed=len(walls), total_wall_seconds=sum(walls), mean_wall_seconds=sum(walls)/len(walls) if walls else None)
    summary = dict(non_compared_sizing_policy=turn.sizing_policy(args), identity=manifest['identity'], spots=spots, sources=sources, failures=failures,
                   completed=sum(s['complete'] for s in spots), total=len(spots), winner_counts=counts,
                   timing=timings, upper_bound=estimate_counts(len(manifest['provenance']['selection']), args.max_turns), no_optimality_claim=True)
    save(args.out/'summary.json', summary)
    lines = ['# M4 river sizing comparison', '',
             'Non-compared sizing policy: '+json.dumps(summary['non_compared_sizing_policy'], sort_keys=True), '',
             'Bettor root EV: OOP=0 lead; IP=1 after check. Not conditional action EV.',
             'All four candidates use the river target; no coarse elimination.',
             'Loss = (best observed bettor EV - candidate EV) / river starting pot × 100.',
             'No optimality claim or independent holdout validation; raked exploitability is not an EV bound.', '',
             '| Flop / features | Flop line | Turn / kind | Turn line | River / kind | Bettor | Pot BB | Size % | EV BB | Loss % pot | Exploit % | Wall sec | Status |',
             '|---|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|---|']
    for s in spots:
        for r in s['candidates']:
            lines.append(f"| {s['flop']} {json.dumps(s['features'], sort_keys=True)} | {s['flop_line']} | {s['turn_card']} / {s['turn_kind']} | {s['turn_line']} | {s['river_card']} / {s['river_kind']} | {s['bettor']} | {s['river_pot_bb']} | {r['size']} | {r.get('bettor_ev_bb', '—')} | {r.get('loss_pct_pot', '—')} | {r.get('exploitability_pct', '—')} | {r.get('resource', {}).get('wall_seconds', '—')} | {r['status']} |")
    for title, value in [('Counts', dict(completed=summary['completed'], total=summary['total'], winner_counts=counts, upper_bound=summary['upper_bound'])), ('Timing', timings), ('Source solves', sources), ('Failures', failures), ('Provenance', manifest)]:
        lines += ['', '## '+title, '```json', json.dumps(value, indent=2), '```']
    lines += ['', 'Timing includes successful subprocess wall times, including cached evidence, not elapsed time of this invocation.',
              'For a real 1-flop pilot, use actual supplied rules/ranges, --flops BOARD, --max-turns 3 and a new output directory.',
              'Extrapolate N × (mean flop wall + 6 × mean turn wall + 480 × mean river wall) only if all phases have real measurements.',
              'This is a 5 river-kind upper-bound model; empty buckets reduce counts. No production timing is inferred from synthetic inputs.']
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text('\n'.join(lines)+'\n')
    return summary


def main(argv=None):
    global SIZES
    parser = argparse.ArgumentParser(description=__doc__)
    for name, default in [('binary', ROOT/'target/release/cli'), ('sizing-binary', ROOT/'target/release/sizing'), ('out', ROOT/'data/m4-river'), ('report', ROOT/'docs/benchmarks/m4-river.md'), ('flop-rule', ROOT/'configs/sizing/srp-btn-vs-bb.json'), ('manifest', ROOT/'data/m4b/manifest.json'), ('oop-range', ROOT/'data/ranges/bb_call.json'), ('ip-range', ROOT/'data/ranges/btn_open.json')]:
        parser.add_argument('--'+name, type=Path, default=default)
    parser.add_argument('--other-side-rules', type=Path, help='full rules for all non-compared decisions')
    parser.add_argument('--flops', default='20')
    parser.add_argument('--lines', default='XX,BC')
    parser.add_argument('--sizes', default=','.join(map(str, SIZES)), help='candidate bet sizes in percent of the pot')
    parser.add_argument('--turn-kinds', default=','.join(TURN_KINDS))
    for name, default in [('threads', 4), ('memory-mb', 16384), ('max-iters', 10000), ('max-turns', 3), ('per-kind', 1), ('seed', 20261015)]:
        parser.add_argument('--'+name, type=int, default=default)
    for name, default in [('flop-target', 1.), ('turn-target', .5), ('river-target', .1), ('max-hours', 8.), ('max-solve-seconds', 7200.), ('pot', 5.5), ('stack', 97.5)]:
        parser.add_argument(*(['--river-target', '--target'] if name == 'river-target' else ['--'+name]), type=float, default=default)
    args = parser.parse_args(argv)
    args.lines = args.lines.split(',')
    # Candidates are part of the run's identity through vars(args).
    SIZES = tuple(int(x) for x in args.sizes.split(','))
    turn.SIZES = SIZES
    if len(SIZES) < 2 or len(set(SIZES)) != len(SIZES) or min(SIZES) <= 0:
        parser.error('--sizes needs at least two distinct positive sizes')
    args.turn_kinds = args.turn_kinds.split(',')
    if not args.lines or len(set(args.lines)) != len(args.lines) or any(x not in LINES for x in args.lines):
        parser.error('--lines must contain unique XX and/or BC')
    if not args.turn_kinds or len(set(args.turn_kinds)) != len(args.turn_kinds) or any(x not in TURN_KINDS for x in args.turn_kinds):
        parser.error('invalid or duplicate --turn-kinds')
    if min(args.threads, args.memory_mb, args.max_iters, args.max_turns) <= 0 or args.per_kind != 1 or not 0 <= args.seed < 2**64 or any(not math.isfinite(x) or x <= 0 for x in (args.flop_target, args.turn_target, args.river_target, args.max_hours, args.max_solve_seconds, args.pot, args.stack)):
        parser.error('positive finite resources required; --per-kind must be 1; seed must fit u64')
    for key, value in vars(args).items():
        if isinstance(value, Path):
            setattr(args, key, value.resolve())
    try:
        selection = turn.select_flops(args.flops, args.manifest)
        paths = [args.binary, args.sizing_binary, args.other_side_rules or args.flop_rule, args.oop_range, args.ip_range, Path(__file__).resolve(), ROOT/'tools/m4_turn.py', ROOT/'tools/m3_bench.py', ROOT/'tools/m4_trial.py']
        if args.flops.isdigit():
            paths.append(args.manifest)
        paths += sorted(ROOT.glob('crates/**/*.rs')) + sorted(ROOT.glob('crates/**/Cargo.toml')) + [ROOT/'Cargo.lock', ROOT/'Cargo.toml']
        config = {k: str(v) if isinstance(v, Path) else v for k, v in vars(args).items() if k not in ('out', 'report', 'max_hours')}
        provenance = dict(non_compared_sizing_policy=turn.sizing_policy(args), hashes={str(p): m3_digest(p) for p in paths}, config=config, selection=selection)
    except (OSError, ValueError, KeyError) as error:
        parser.error(str(error))
    identity = fingerprint(provenance)
    args.out.mkdir(parents=True, exist_ok=True)
    path = args.out/'manifest.json'
    if path.exists():
        manifest = json.loads(path.read_text())
        if manifest['identity'] != identity:
            parser.error('resume provenance mismatch; use a new --out directory')
    else:
        manifest = dict(identity=identity, provenance=provenance, created_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(), budget_deadline_unix=time.time()+args.max_hours*3600)
        save(path, manifest)
    deadline = time.monotonic()+max(0, min(args.max_hours*3600, manifest['budget_deadline_unix']-time.time()))
    spots, failures, sources = [], [], []
    for selected in selection:
        board = selected['flop']
        record = card_selection(args, board, 'turn', identity, deadline)
        if record['status'] != 'complete':
            failures.append(dict(flop=board, phase='turn-cards', record=record)); continue
        cards = select_turns(record['cards'], args.turn_kinds, args.max_turns)
        if not any(g['cards'] for g in cards['groups']):
            failures.append(dict(flop=board, phase='turn-cards', error='no selected turn cards')); continue
        flop = solve_source(args, board, cards, identity, deadline)
        sources.append(dict(phase='flop', board=board, record=flop))
        if flop['status'] != 'complete':
            failures.append(dict(flop=board, phase='flop', record=flop)); continue
        for group in cards['groups']:
            for card in group['cards']:
                river_cards = card_selection(args, board+card, 'river', identity, deadline)
                if river_cards['status'] != 'complete':
                    failures.append(dict(board=board+card, phase='river-cards', record=river_cards)); continue
                for flop_line in args.lines:
                    state = Path(flop['reach_directory'])/f'{flop_line}-{card}.json'
                    source_id = f'{board}-{flop_line}-{card}'
                    source = solve_source(args, board+card, river_cards['cards'], identity, deadline, state, source_id)
                    sources.append(dict(phase='turn', board=board+card, flop_line=flop_line, record=source))
                    if source['status'] != 'complete':
                        failures.append(dict(id=source_id, phase='turn', record=source)); continue
                    for turn_line in args.lines:
                        for river_group in river_cards['cards']['groups']:
                            for river_card in river_group['cards']:
                                river_state = Path(source['reach_directory'])/f'{turn_line}-{river_card}.json'
                                snapshot = json.loads(river_state.read_text())
                                for bettor in (0, 1):
                                    spot = dict(id=f'{source_id}-{turn_line}-{river_card}-p{bettor}', flop=board,
                                                features=selected.get('features', cards.get('features', {})), flop_line=flop_line,
                                                turn_card=card, turn_kind=group['kind'], turn_line=turn_line,
                                                river_card=river_card, river_kind=river_group['kind'], bettor=bettor,
                                                after_check=bettor == 1, river_pot_bb=snapshot['pot']/snapshot['units_per_bb'])
                                    spots.append(compare(args, spot, river_state, identity, deadline))
    summary = summarize(args, manifest, spots, failures, sources)
    print(json.dumps(dict(completed=summary['completed'], total=summary['total'], failures=len(failures), report=str(args.report))))
    return 0 if not failures and spots and all(s['complete'] for s in spots) else 1


if __name__ == '__main__':
    sys.exit(main())
