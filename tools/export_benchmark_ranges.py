#!/usr/bin/env python3
"""Read saved product JSON only; export exact benchmark frequencies locally.

No product files are changed. Output belongs under ignored data/ranges, never in
Git. Class frequencies are applied to EACH combo, not divided by class size.
"""
import argparse
import csv
import hashlib
import json
import math
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PRODUCT = Path('/Users/yota/Projects/Products/ReysonAI')
RANKS, SUITS = '23456789TJQKA', 'cdhs'


def card_id(card):
    if len(card) != 2 or card[0] not in RANKS or card[1] not in SUITS:
        raise ValueError(f'invalid card: {card}')
    return RANKS.index(card[0]) * 4 + SUITS.index(card[1])


def canonical(combo):
    if len(combo) != 4:
        raise ValueError('expected explicit two-card combo')
    cards = [combo[:2], combo[2:]]
    if cards[0] == cards[1]:
        raise ValueError('duplicate card')
    return ''.join(sorted(cards, key=card_id))


def expand_class(hand):
    if len(hand) not in (2, 3) or hand[0] not in RANKS or hand[1] not in RANKS:
        raise ValueError('invalid class')
    pair = hand[0] == hand[1]
    if (pair and len(hand) != 2) or (not pair and (len(hand) != 3 or hand[2] not in 'so')):
        raise ValueError('invalid class suffix')
    return [canonical(hand[0] + s + hand[1] + t)
            for i, s in enumerate(SUITS) for j, t in enumerate(SUITS)
            if (i < j if pair else ((s == t) == (hand[2] == 's')))]


def frequency(value):
    value = float(value)
    if not math.isfinite(value) or not 0 <= value <= 1:
        raise ValueError('frequency must be finite and in [0,1]')
    return value


def insert(weights, combo, weight):
    combo = canonical(combo)
    if combo in weights:
        raise ValueError('duplicate combo')
    weights[combo] = frequency(weight)


def bb_weights(data, node_id):
    nodes = [n for n in data['nodes'] if n['nodeId'] == node_id]
    if len(nodes) != 1:
        raise ValueError('expected exactly one BB source node')
    node = nodes[0]
    if node['actingPosition'] != 'BB' or node['effectiveStackBb'] != 97.5:
        raise ValueError('unexpected BB source context')
    if node['actionHistory']['actions'] != [
        {'position': 'BTN', 'action': {'type': 'raise', 'sizeBb': 2.5}},
        {'position': 'SB', 'action': {'type': 'fold'}}]:
        raise ValueError('unexpected BB source history')
    weights = {}
    for row in node['combos']:
        calls = [a for a in row['actions'] if a['action'] == 'call']
        if len(calls) != 1:
            raise ValueError('expected exactly one call frequency')
        insert(weights, row['combo'], calls[0]['frequency'])
    return {k: v for k, v in weights.items() if v > 0}


def btn_weights(data):
    spots = [s for s in data['spots'] if s['id'] == 'BTN_open'
             and s['effective_stack_bb'] == 100 and s['open_size_bb'] == 2.5]
    if len(spots) != 1:
        raise ValueError('expected exactly one BTN_open 100BB/2.5BB row')
    weights = {}
    for row in spots[0]['hands']:
        value = frequency(float(row['open']) / 100)
        for combo in expand_class(row['hand']):
            insert(weights, combo, value)
    return {k: v for k, v in weights.items() if v > 0}


def blocked(weights, board):
    cards = {board[i:i + 2] for i in range(0, len(board), 2)}
    return {c: w for c, w in weights.items() if not ({c[:2], c[2:]} & cards)}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_source(path, expected_sha):
    actual = sha(path)
    if actual != expected_sha:
        raise ValueError(f'source SHA256 mismatch: {path}; expected {expected_sha}, got {actual}')
    return json.loads(path.read_text()), actual


def compare_reference(bb, btn, benchmark):
    """Root reach is own frequency times compatible opponent starting mass."""
    with (benchmark / 'strategy-root-oop-bb-combos.csv').open(newline='') as stream:
        rows = list(csv.DictReader(stream))
    actual = {canonical(r['combo']): float(r['reach_weight']) for r in rows}
    if len(rows) != len(actual) or set(actual) != set(bb):
        raise ValueError('BB reference combo coverage mismatch')
    maximum = 0.0
    for combo, weight in bb.items():
        cards = {combo[:2], combo[2:]}
        opponent_mass = math.fsum(w for c, w in btn.items() if not (cards & {c[:2], c[2:]}))
        expected = weight * opponent_mass
        error = abs(actual[combo] - expected)
        maximum = max(maximum, error)
        # Saved outputs have float32 reach arithmetic and nine decimal places.
        if not math.isclose(actual[combo], expected, rel_tol=1e-6, abs_tol=1e-7):
            raise ValueError(f'BB reference compatible reach mismatch: {combo}')
    with (benchmark / 'strategy-after-check-ip-btn-combos.csv').open(newline='') as stream:
        rows = list(csv.DictReader(stream))
    combos = [canonical(r['combo']) for r in rows]
    if len(combos) != len(set(combos)) or set(combos) != set(btn):
        raise ValueError('BTN reference combo coverage mismatch')
    return {'bb_root_reach_max_absolute_error': maximum,
            'bb_root_reach_relative_tolerance': 1e-6,
            'bb_root_reach_absolute_tolerance': 1e-7,
            'btn_after_check_combo_coverage': len(combos)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bb-source', type=Path, default=PRODUCT / 'solutions/cash-6max-100bb-v1.json')
    parser.add_argument('--btn-source', type=Path, default=PRODUCT / 'apps/frontend/src/estimated/opening-ranges.json')
    parser.add_argument('--benchmark', type=Path, default=ROOT / 'data/benchmark')
    parser.add_argument('--out-dir', type=Path, default=ROOT / 'data/ranges')
    args = parser.parse_args()
    # Prevent accidental export of product data outside the ignored data tree.
    if not args.out_dir.resolve().is_relative_to((ROOT / 'data').resolve()):
        parser.error('--out-dir must be under the ignored repository data directory')
    preflight = json.loads((args.benchmark / 'tree-preflight.json').read_text())
    sources = preflight['input_ranges']
    raw_bb, bb_sha = load_source(args.bb_source, sources['BB_call']['file_sha256'])
    raw_btn, btn_sha = load_source(args.btn_source, sources['BTN_open']['file_sha256'])
    ranges = {'bb_call': bb_weights(raw_bb, sources['BB_call']['node_id']), 'btn_open': btn_weights(raw_btn)}
    bb, btn = (blocked(ranges[name], preflight['board']) for name in ('bb_call', 'btn_open'))
    if (len(bb), len(btn)) != (1176, 690) or preflight['live_combos_after_board_blocking'] != {'BB_oop': 1176, 'BTN_ip': 690}:
        raise ValueError('fixed benchmark live-combo counts mismatch')
    proof = compare_reference(bb, btn, args.benchmark)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    files = {}
    for name, weights in ranges.items():
        target = args.out_dir / f'{name}.json'
        target.write_text(json.dumps({'player': name, 'combos': dict(sorted(weights.items()))}, indent=2) + '\n')
        files[name] = {'file': target.name, 'sha256': sha(target), 'positive_combos': len(weights)}
    provenance = {'warning': sources['warning'], 'board': preflight['board'],
                  'source_sha256': {'BB_call': bb_sha, 'BTN_open': btn_sha},
                  'preflight_sha256': sha(args.benchmark / 'tree-preflight.json'),
                  'source_range_sha256_as_recorded': {name: sources[name]['range_sha256'] for name in ('BB_call', 'BTN_open')},
                  'range_sha_note': 'Recorded upstream range fingerprints; upstream serialization is not reimplemented.',
                  'weight_semantics': 'Exact BB combo call frequency; BTN class open/100 copied equally to EACH class combo. No renormalization or suit symmetrization.',
                  'live_combos_after_board_blocking': {'BB_oop': len(bb), 'BTN_ip': len(btn)},
                  'files': files, 'reference_validation': proof}
    (args.out_dir / 'provenance.json').write_text(json.dumps(provenance, indent=2) + '\n')
    print(json.dumps({'live_combos': provenance['live_combos_after_board_blocking'],
                      'source_sha256_match': True, 'reference_validation': proof,
                      'output_dir': str(args.out_dir)}, indent=2))


if __name__ == '__main__':
    main()
