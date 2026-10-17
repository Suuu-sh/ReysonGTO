"""Small synthetic runner tests; no production solves."""
import json
from pathlib import Path
import sys
import time
import tempfile
import unittest
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parent))
import m4_turn as turn


def test_rules_isolate_bettor_and_street():
    for player in (0, 1):
        table = turn.candidate_rules(125, player)
        assert table['default_fraction'] == .66
        assert table['rules'][0]['street'] == 'Turn'
        assert table['rules'][0]['player'] == player
        assert table['rules'][0]['after_check'] == (player == 1)
        assert table['rules'][0]['fraction'] == 1.25
    table = turn.flop_rules(dict(version=1, candidates=[.33], default_fraction=.33,
                                rules=[dict(player=1, fraction=.5), dict(street='River', fraction=1.)]), {})
    assert all(r['street'] == 'Flop' for r in table['rules'])
    assert table['default_fraction'] == .66
    assert table['rules'][2]['fraction'] == .5
    tie = turn.flop_rules(dict(version=1, candidates=[.33], default_fraction=.33, rules=[dict(street='Flop', fraction=.8), dict(player=1, fraction=.5)]), {})
    assert tie['rules'][2]['fraction'] == .8


def test_attempt_resume_integrity_and_failure_retained(tmp_path):
    calls = []
    def build(directory):
        calls.append(directory)
        command = [sys.executable, '-c', 'from pathlib import Path; Path('+repr(str(directory/'solution.json'))+').write_text("{}")']
        return command, [directory/'solution.json']
    validate = lambda d: dict(target_met=True)
    d = tmp_path/'run'
    first = turn.run_attempt(d, 'identity', time.monotonic()+10, build, validate)
    second = turn.run_attempt(d, 'identity', time.monotonic()+10, build, validate)
    assert first == second and len(calls) == 1
    Path(next(iter(first['artifacts']))).write_text('changed')
    third = turn.run_attempt(d, 'identity', time.monotonic()+10, build, validate)
    assert third['status'] == 'complete' and len(calls) == 2
    def fail(_):
        raise ValueError('synthetic failure')
    bad = turn.run_attempt(d, 'different', time.monotonic()+10, build, fail)
    assert bad['status'] == 'invalid_or_incomplete'
    assert (d/'attempt-1/result.json').exists()
    assert (d/'attempt-3/result.json').exists()


def test_compare_uses_bettor_ev_and_requires_targets(tmp_path):
    args = SimpleNamespace(out=tmp_path)
    spot = dict(id='synthetic', bettor=0)
    def candidate(args, spot, state, stage, size, identity, deadline):
        value = size if spot['bettor'] == 0 else -size
        return dict(size=size, stage=stage, status='complete', target_met=True,
                    bettor_ev_bb=value, diagnostic_proxy_bb=.1)
    original = turn.candidate
    turn.candidate = candidate
    result = turn.compare(args, spot, None, 'id', 0)
    assert result['observed_winner'] == 125
    result = turn.compare(args, dict(spot, bettor=1), None, 'id', 0)
    assert result['observed_winner'] == 33
    def unconverged(*args):
        return dict(candidate(*args), target_met=False)
    turn.candidate = unconverged
    result = turn.compare(args, spot, None, 'id', 0)
    assert result['observed_winner'] is None and result['fine'] == []
    turn.candidate = original


def test_select_flops_and_metrics(tmp_path):
    manifest = tmp_path/'manifest.json'
    manifest.write_text(json.dumps(dict(selection=dict(training=[dict(flop='QsJh2h'), dict(flop='As7d2c')]))))
    assert turn.select_flops('1', manifest) == [dict(flop='QsJh2h')]
    assert turn.select_flops('QsJh2h,As7d2c', manifest)[1]['flop'] == 'As7d2c'
    report = tmp_path/'solution.json'
    report.write_text(json.dumps(dict(game_spec=dict(units_per_bb=1000, tree=dict(starting_pot=2000)),
        metrics=dict(average_ev=[-20, 10], best_response_ev=[0, 30], exploitability_pct=1),
        iterations=10, solve_seconds=.1, stopped_by='target')))
    assert turn.read_metrics(report, 1, 0)['bettor_ev_bb'] == -.02
    assert turn.read_metrics(report, 1, 1)['bettor_ev_bb'] == .01


class RunnerTests(unittest.TestCase):
    def test_rules(self):
        test_rules_isolate_bettor_and_street()

    def test_resume(self):
        with tempfile.TemporaryDirectory() as d:
            test_attempt_resume_integrity_and_failure_retained(Path(d))

    def test_bettor(self):
        with tempfile.TemporaryDirectory() as d:
            test_compare_uses_bettor_ev_and_requires_targets(Path(d))

    def test_unconverged_retried(self):
        with tempfile.TemporaryDirectory() as d:
            directory = Path(d)
            def build(attempt):
                return [sys.executable, '-c', 'pass'], [attempt/'stdout.log']
            first = turn.run_attempt(directory, 'id', time.monotonic()+5, build, lambda _: dict(target_met=False, bettor_ev_bb=1.))
            assert first['status'] == 'target_not_met' and first['bettor_ev_bb'] == 1.
            second = turn.run_attempt(directory, 'id', time.monotonic()+5, build, lambda _: dict(target_met=True))
            assert second['status'] == 'complete' and (directory/'attempt-2').exists()

    def test_summary_keeps_bettors_separate(self):
        with tempfile.TemporaryDirectory() as d:
            args = SimpleNamespace(out=Path(d), report=Path(d)/'report.md')
            spots = [dict(id=str(p), kind='Blank', bettor=p, complete=True, observed_winner=size,
                          coarse=[], fine=[]) for p, size in [(0, 33), (1, 125)]]
            summary = turn.summarize(args, dict(identity='id'), spots, [])
            assert summary['winner_counts'] == {'Blank/p0': {'33': 1}, 'Blank/p1': {'125': 1}}
            assert 'Completed situations: 2 / 2' in args.report.read_text()

    def test_chips_match_cli_rounding(self):
        self.assertEqual(turn.chips(5.5), 5500)
        self.assertEqual(turn.chips(5.5004), 5500)
        self.assertEqual(turn.chips(2.5005), 2501)

    def test_metrics(self):
        with tempfile.TemporaryDirectory() as d:
            test_select_flops_and_metrics(Path(d))


if __name__ == '__main__':
    unittest.main()
