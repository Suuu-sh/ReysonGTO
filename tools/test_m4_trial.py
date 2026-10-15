import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import m4_trial as trial


class TrialTests(unittest.TestCase):
    def test_proxy_is_sum_and_not_ev_bound(self):
        a = dict(btn_ev_bb=1, diagnostic_proxy_bb=.22)
        b = dict(btn_ev_bb=1.3, diagnostic_proxy_bb=.11)
        result = trial.diagnostic(a, b)
        self.assertTrue(result['indistinguishable_by_proxy'])
        self.assertAlmostEqual(result['combined_proxy_bb'], .33)
        self.assertIn('NOT', result['interpretation'])

    def test_timeout_reaps_child_and_keeps_log(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            resource = trial.m3.run_child([sys.executable, '-c', 'import time; print("partial", flush=True); time.sleep(5)'], directory, .2)
            self.assertTrue(resource['timed_out'])
            with self.assertRaises(ChildProcessError):
                os.waitpid(resource['solver_pid'], os.WNOHANG)
            self.assertIn('partial', (directory / 'stdout.log').read_text())

    def test_resume_checks_identity_and_result_hash(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'solution.json'
            path.write_text('{}')
            r = dict(identity='a', status='complete', solution_path=str(path), solution_sha256=trial.m3.digest(path))
            self.assertTrue(trial.reusable(r, 'a'))
            self.assertFalse(trial.reusable(r, 'b'))
            path.write_text('{"changed":true}')
            self.assertFalse(trial.reusable(r, 'a'))

    def test_exact_metrics_units_and_invalid(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'solution.json'
            report = dict(metrics=dict(average_ev=[1000, 2000], best_response_ev=[1110, 2110], exploitability_pct=2), iterations=10, solve_seconds=1, stopped_by='target')
            path.write_text(json.dumps(report))
            r = trial.metrics(path, 2)
            self.assertEqual(r['btn_ev_bb'], 2)
            self.assertAlmostEqual(r['diagnostic_proxy_bb'], .22)
            self.assertTrue(r['target_met'])
            report['metrics']['average_ev'][1] = float('nan')
            path.write_text(json.dumps(report))
            with self.assertRaises(ValueError):
                trial.metrics(path, 2)

    def test_fake_child_nonzero_and_invalid_json(self):
        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp)
            r = trial.m3.run_child([sys.executable, '-c', 'raise SystemExit(7)'], d, 2)
            self.assertEqual(r['exit_code'], 7)
            (d / 'solution.json').write_text('invalid')
            with self.assertRaises(ValueError):
                trial.metrics(d / 'solution.json', 2)

    def test_wrong_board_rejected(self):
        from types import SimpleNamespace
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'solution.json'
            path.write_text(json.dumps(dict(game_spec=dict(board=[0, 1, 2], units_per_bb=1000, tree={}))))
            with self.assertRaisesRegex(ValueError, 'specification'):
                trial.validate_solution(path, 'AcKdJh', 33, SimpleNamespace(threads=10))

    def test_board_retains_eliminated_coarse_and_overlap(self):
        def row(size, ev, stage=1):
            return dict(size=size, btn_ev_bb=ev, stage=stage, diagnostic_proxy_bb=.5, status='complete')
        r = trial.board_result(dict(flop='AcKdJh', features={}), [row(33, 1), row(66, 1.1), row(100, 1.2), row(125, 1.3)], [row(125, 1.4, 2), row(100, 1.35, 2)], 'id')
        self.assertTrue(r['complete'])
        self.assertEqual(len(r['stage1']), 4)
        eliminated = [c for c in r['comparisons'] if c['eliminated_unrefined']]
        self.assertEqual(len(eliminated), 2)
        self.assertTrue(all(c['indistinguishable_by_proxy'] for c in eliminated))


if __name__ == '__main__':
    unittest.main()
