"""Focused synthetic runner checks; no solver, product range, or benchmark runs."""
import contextlib
import io
import json
import os
from pathlib import Path
import signal
import sys
import tempfile
import unittest

from m3_bench import BOARDS, btn_first_strategy, checkpoints, main


FAKE = '''#!{python}
import argparse, json, os, sys, time
from pathlib import Path
p = argparse.ArgumentParser()
p.add_argument('--board')
p.add_argument('--out', type=Path)
p.add_argument('--progress-jsonl', type=Path)
a, _ = p.parse_known_args()
marker = Path(__file__).with_suffix('.previous-pid')
if marker.exists():
    try:
        os.kill(int(marker.read_text()), 0)
    except ProcessLookupError:
        pass
    else:
        sys.exit(99) # Runner must reap the previous exact child first.
marker.write_text(str(os.getpid()))
record = dict(iteration=5, elapsed_seconds=0.01,
              metrics=dict(exploitability_pct=1.25))
a.progress_jsonl.write_text(json.dumps(dict(phase='checkpoint', record=record)) + '\\n')
mode = Path(__file__).stem
if mode == 'mixed' and a.board == 'As7d2c':
    sys.exit(7)
if mode in ('mixed', 'timeout') and a.board == 'Th9h8c':
    with a.progress_jsonl.open('a') as stream:
        stream.write('{{"partial":') # Preserve a partial final progress line.
    time.sleep(10)
strategy = dict(node=1, player=1, actions=['Check', {{'Bet': 1815}}],
    classes={{'AA': [0.35, 0.65]}}, combos=[
        dict(combo='AcAd', weight=1.0, reach_weight=3.0, probabilities=[0.2, 0.8]),
        dict(combo='AhAs', weight=3.0, reach_weight=1.0, probabilities=[0.8, 0.2])])
report = dict(iterations=17, setup_seconds=0.001, solve_seconds=0.012,
              training_seconds=0.009, metrics_seconds=0.003, stopped_by='target',
              metrics=dict(exploitability_pct=0.29), after_check_strategy=strategy)
a.out.write_text(json.dumps(report))
'''


class RunnerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.directory = Path(self.tmp.name)
        self.oop = self.directory / 'synthetic-oop.json'
        self.ip = self.directory / 'synthetic-ip.json'
        self.oop.write_text('{}')
        self.ip.write_text('{}')

    def tearDown(self):
        self.tmp.cleanup()

    def binary(self, mode):
        path = self.directory / mode
        path.write_text(FAKE.format(python=sys.executable))
        path.chmod(0o700)
        return path

    def run_bench(self, binary, out, extra=()):
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            return main(['--binary', str(binary), '--oop-range', str(self.oop),
                         '--ip-range', str(self.ip), '--out', str(out), *extra])

    def test_success_manifest_exact_rss_and_distinct_strategy_weightings(self):
        binary = self.binary('success')
        out = self.directory / 'success-output'
        self.assertEqual(self.run_bench(binary, out), 0)
        summary = json.loads((out / 'summary.json').read_text())
        self.assertTrue(summary['all_complete'] and summary['all_targets_met'])
        self.assertEqual([row['board'] for row in summary['boards']], list(BOARDS))
        for row in summary['boards']:
            self.assertEqual(row['status'], 'complete')
            self.assertEqual(row['final_exploitability_pct'], 0.29)
            self.assertEqual(row['iterations'], 17)
            self.assertAlmostEqual(row['btn_bet_frequency_pct'], 65)
            self.assertAlmostEqual(row['btn_bet_frequency_own_reach_pct'], 35)
            self.assertGreater(row['resource']['maximum_resident_set_bytes'], 0)
            self.assertIn('exact solver PID', row['resource']['source'])
            with self.assertRaises(ProcessLookupError):
                os.kill(row['resource']['solver_pid'], 0)
            strategy = json.loads((Path(row['directory']) / 'btn-first-strategy.json').read_text())
            self.assertEqual(strategy['classes'], {'AA': [0.35, 0.65]})
            self.assertEqual(strategy['primary_weighting'], 'compatible_joint')
        manifest = json.loads((out / 'manifest.json').read_text())
        self.assertEqual(len(manifest['binary_sha256']), 64)
        self.assertEqual(set(manifest['inputs']), {str(self.ip.resolve()), str(self.oop.resolve())})
        self.assertIn('NOT binary build provenance', manifest['source_note'])
        for command in manifest['commands']:
            def option(name):
                return command[command.index(name) + 1]
            self.assertEqual(option('--preset'), 'production')
            self.assertEqual(option('--storage'), 'f32')
            self.assertEqual(option('--dcfr-gamma'), '2')
            self.assertEqual(option('--rake'), '0.05')
            self.assertEqual(option('--bet-flop'), '33')
            self.assertEqual(option('--bet-turn'), '66')
            self.assertEqual(option('--bet-river'), '66')
            self.assertEqual(option('--max-seconds'), '1740')
            self.assertIn('--no-oop-leads', command)
            self.assertIn('--allin-merge', command)

    def test_failure_and_timeout_keep_last_checkpoint_not_a_final_result(self):
        out = self.directory / 'mixed-output'
        self.assertEqual(self.run_bench(self.binary('mixed'), out,
                                       ['--max-seconds', '0.01', '--timeout-seconds', '0.5']), 1)
        rows = json.loads((out / 'summary.json').read_text())['boards']
        self.assertEqual([row['status'] for row in rows], ['complete', 'process_error', 'timeout'])
        self.assertEqual(rows[1]['resource']['exit_code'], 7)
        self.assertEqual(rows[2]['resource']['exit_code'], 124)
        self.assertEqual(rows[2]['resource']['child_exit_code'], -signal.SIGKILL)
        self.assertTrue(rows[2]['resource']['timed_out'])
        self.assertEqual(len(rows[2]['progress_parse_errors']), 1)
        for row in rows[1:]:
            self.assertIsNone(row['final_exploitability_pct'])
            self.assertIsNone(row['iterations'])
            self.assertIsNone(row['target_met'])
            self.assertFalse(row['final_result_accepted'])
            self.assertEqual(row['last_checkpoint']['iteration'], 5)
            self.assertEqual(row['last_checkpoint']['metrics']['exploitability_pct'], 1.25)
            self.assertFalse((Path(row['directory']) / 'btn-first-strategy.json').exists())
            with self.assertRaises(ProcessLookupError):
                os.kill(row['resource']['solver_pid'], 0)

    def test_timeout_child_is_reaped_before_next_board(self):
        out = self.directory / 'timeout-then-next'
        self.assertEqual(self.run_bench(self.binary('timeout'), out,
                                       ['--boards', 'Th9h8c', 'Qh8d2c',
                                        '--max-seconds', '0.01', '--timeout-seconds', '0.5']), 1)
        rows = json.loads((out / 'summary.json').read_text())['boards']
        self.assertEqual([row['status'] for row in rows], ['timeout', 'complete'])

    def test_existing_output_is_untouched_and_no_child_launched(self):
        binary = self.binary('success')
        out = self.directory / 'already-exists'
        out.mkdir()
        sentinel = out / 'keep.txt'
        sentinel.write_text('preserve')
        with self.assertRaises(SystemExit) as caught:
            self.run_bench(binary, out)
        self.assertEqual(caught.exception.code, 2)
        self.assertEqual(sentinel.read_text(), 'preserve')
        self.assertFalse(binary.with_suffix('.previous-pid').exists())

    def test_invalid_limits_are_rejected_before_output_creation(self):
        binary = self.binary('success')
        for options in [['--timeout-seconds', 'nan'], ['--max-seconds', '1800'],
                        ['--threads', '0'], ['--boards', 'Qh8d2c', 'Qh8d2c']]:
            out = self.directory / 'invalid'
            with self.assertRaises(SystemExit) as caught:
                self.run_bench(binary, out, options)
            self.assertEqual(caught.exception.code, 2)
            self.assertFalse(out.exists())

    def test_missing_progress_and_invalid_strategy(self):
        self.assertEqual(checkpoints(self.directory / 'absent.jsonl'), ([], []))
        with self.assertRaisesRegex(ValueError, 'BTN'):
            btn_first_strategy({'after_check_strategy': None}, BOARDS[0])


if __name__ == '__main__':
    unittest.main()
