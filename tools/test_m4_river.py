"""Tiny synthetic river comparison tests. Never reads production ranges."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import m4_river as river
import m4_turn as turn
from test_m4_turn import check_other_side_policy

ROOT = Path(__file__).resolve().parents[1]


class RunnerTests(unittest.TestCase):
    def test_other_side_policy(self):
        check_other_side_policy(self, river, 'River')

    def test_candidate_bettor_and_street(self):
        for bettor in (0, 1):
            table = river.candidate_rules(125, bettor)
            self.assertEqual(table['default_fraction'], .66)
            self.assertEqual(len(table['rules']), 1)
            row = table['rules'][0]
            self.assertEqual((row['street'], row['player'], row['after_check'], row['fraction']),
                             ('River', bettor, bettor == 1, 1.25))

    def test_supplied_flop_and_turn_rules(self):
        table = dict(version=1, candidates=[.33,.66,1.,1.25], default_fraction=.33,
                     rules=[dict(street='Flop', fraction=1.25),
                            dict(street='Turn', player=1, after_check=True, fraction=1.),
                            dict(player=0, fraction=.5), dict(street='River', fraction=1.25)])
        result = river.source_rules(table)
        self.assertEqual(result['default_fraction'], .66)
        self.assertFalse(any(r['street'] == 'River' for r in result['rules']))
        for street, player, after, expected in [('Flop',1,True,1.25), ('Turn',1,True,1.), ('Turn',0,False,.5)]:
            matches = [(sum(v is not None for k,v in row.items() if k not in ('fraction','sources')), i, row['fraction'])
                       for i,row in enumerate(result['rules'])
                       if all(v is None or {'street':street,'player':player,'after_check':after,'previous_street_line':'checked_through','runout':'Blank'}.get(k)==v
                              for k,v in row.items() if k not in ('fraction','sources'))]
            self.assertEqual(sorted(matches, key=lambda x:(-x[0],x[1]))[0][2], expected)

    def test_source_rule_ties_and_previous_street(self):
        def choose(table,context):
            matches=[(sum(v is not None for k,v in row.items() if k not in ('fraction','sources')),i,row['fraction'])
                     for i,row in enumerate(table['rules'])
                     if all(v is None or context.get(k)==v for k,v in row.items() if k not in ('fraction','sources'))]
            return sorted(matches,key=lambda x:(-x[0],x[1]))[0][2] if matches else table['default_fraction']
        explicit=dict(street='Turn',fraction=1.25)
        wildcard=dict(player=1,fraction=.33)
        for rows in [[explicit,wildcard],[wildcard,explicit]]:
            table=dict(version=1,candidates=[.33,.66,1.,1.25],default_fraction=.66,rules=rows)
            context=dict(street='Turn',player=1,after_check=True,previous_street_line='checked_through',runout='Blank')
            self.assertEqual(choose(river.source_rules(table),context),choose(table,context))
        table=dict(version=1,candidates=[.33,.66,1.,1.25],default_fraction=.66,
                   rules=[dict(street='Turn',previous_street_line='ip_bet_called',fraction=1.),
                          dict(street='Turn',previous_street_line='checked_through',fraction=.33)])
        for previous,expected in [('ip_bet_called',1.),('checked_through',.33)]:
            self.assertEqual(choose(river.source_rules(table),dict(street='Turn',player=1,after_check=True,previous_street_line=previous,runout='Blank')),expected)

    def test_bettor_ev_losses_and_unconverged(self):
        with tempfile.TemporaryDirectory() as d:
            args = SimpleNamespace(out=Path(d))
            def candidate(args, spot, state, size, identity, deadline):
                return dict(size=size,status='complete',target_met=True,
                            bettor_ev_bb=(size/100 if spot['bettor']==0 else -size/100),
                            diagnostic_proxy_bb=.001)
            with patch.object(river,'candidate',side_effect=candidate):
                for bettor,winner in [(0,125),(1,33)]:
                    result = river.compare(args,dict(id=str(bettor),bettor=bettor,river_pot_bb=10),None,'id',0)
                    self.assertTrue(result['complete'])
                    self.assertEqual(result['observed_winner'],winner)
                    self.assertEqual(len(result['candidates']),4)
                    losses = {r['size']:r['loss_pct_pot'] for r in result['candidates']}
                    self.assertAlmostEqual(losses[winner],0)
                    self.assertAlmostEqual(max(losses.values()),9.2)
            def missed(*a):
                return dict(candidate(*a),target_met=False)
            with patch.object(river,'candidate',side_effect=missed):
                result=river.compare(args,dict(id='bad',bettor=0,river_pot_bb=10),None,'id',0)
                self.assertFalse(result['complete'])
                self.assertIsNone(result['observed_winner'])

    def test_counts_and_turn_selection(self):
        self.assertEqual(river.estimate_counts(20), dict(flops=20, turn_solves=120,
                         river_situations=2400, river_solves=9600, total_solves=9740))
        cards=dict(groups=[dict(kind='Blank',cards=['2c']),dict(kind='BoardPair',cards=[]),
                           dict(kind='Overcard',cards=['Ac'])])
        selected=river.select_turns(cards,['BoardPair','Overcard','Blank'],1)
        self.assertEqual([c for g in selected['groups'] for c in g['cards']],['Ac'])

    def test_attempt_artifact_and_identity(self):
        with tempfile.TemporaryDirectory() as d:
            directory=Path(d)
            def build(attempt):
                return [sys.executable,'-c','print("synthetic")'],[attempt/'stdout.log']
            run=lambda identity: turn.run_attempt(directory,identity,time.monotonic()+5,build,lambda _:dict(target_met=True))
            first=run('one')
            self.assertEqual(run('one'),first)
            Path(next(iter(first['artifacts']))).write_text('corrupted')
            self.assertNotEqual(run('one')['attempt'],first['attempt'])
            third=run('two')
            self.assertTrue(third['attempt'].endswith('attempt-3'))
            self.assertTrue((directory/'attempt-1/result.json').is_file())


class CLIIntegrationTests(unittest.TestCase):
    def test_tiny_real_cli_all_histories_and_resume(self):
        # Fail rather than silently skip when binaries have not been built.
        binary = Path(os.environ.get('M4_TEST_CLI', ROOT/'target/release/cli'))
        sizing = Path(os.environ.get('M4_TEST_SIZING', ROOT/'target/release/sizing'))
        self.assertTrue(binary.is_file(), 'build cli first or set M4_TEST_CLI')
        self.assertTrue(sizing.is_file(), 'build sizing first or set M4_TEST_SIZING')
        with tempfile.TemporaryDirectory(prefix='m4-river-synthetic-') as d:
            d=Path(d)
            for player,combos in [('oop',{'AcAd':1,'KcKd':1}),('ip',{'QcQd':1,'JcJd':1})]:
                (d/f'{player}.json').write_text(json.dumps(dict(player='synthetic-'+player,combos=combos)))
            rules=dict(version=1,candidates=[.33,.66,1.,1.25],default_fraction=.66,
                       rules=[dict(street='Flop',fraction=.33),dict(street='Turn',fraction=.5)])
            (d/'rules.json').write_text(json.dumps(rules))
            out=d/'out'
            command=[sys.executable,str(ROOT/'tools/m4_river.py'),'--binary',str(binary),
                     '--sizing-binary',str(sizing),'--flops','7c6d5d','--max-turns','1',
                     '--flop-rule',str(d/'rules.json'),'--oop-range',str(d/'oop.json'),
                     '--ip-range',str(d/'ip.json'),'--pot','2','--stack','3','--threads','1',
                     '--max-iters','3000','--target','.1','--max-hours','.1','--out',str(out),'--report',str(out/'report.md')]
            started=time.monotonic()
            first=subprocess.run(command,capture_output=True,text=True,timeout=180)
            self.assertEqual(first.returncode,0,first.stdout+first.stderr+'\n'+((out/'summary.json').read_text() if (out/'summary.json').exists() else 'no summary'))
            elapsed=time.monotonic()-started
            summary=json.loads((out/'summary.json').read_text())
            self.assertFalse(summary['failures'])
            self.assertGreater(summary['completed'],0)
            self.assertEqual(summary['completed'],summary['total'])
            self.assertEqual({(s['flop_line'],s['turn_line']) for s in summary['spots']},
                             {('XX','XX'),('XX','BC'),('BC','XX'),('BC','BC')})
            self.assertEqual({s['bettor'] for s in summary['spots']},{0,1})
            for spot in summary['spots']:
                self.assertEqual({r['size'] for r in spot['candidates']},{33,66,100,125})
                for result in spot['candidates']:
                    self.assertTrue(result['target_met'])
                    self.assertLessEqual(result['exploitability_pct'],.1)
                    self.assertAlmostEqual(result['loss_pct_pot'],
                        100*(max(r['bettor_ev_bb'] for r in spot['candidates'])-result['bettor_ev_bb'])/spot['river_pot_bb'])
            self.assertEqual([r['phase'] for r in summary['sources']],['flop','turn','turn'])
            for source in summary['sources']:
                report=json.loads((Path(source['record']['reach_directory'])/'solution.json').read_text())
                table=report['game_spec']['size_rules']
                self.assertEqual(table['default_fraction'],.66)
                self.assertTrue(any(r['street']=='Flop' and r['fraction']==.33 for r in table['rules']))
                self.assertTrue(any(r['street']=='Turn' and r['fraction']==.5 for r in table['rules']))
            attempts=list(out.rglob('attempt-*/result.json'))
            second=subprocess.run(command,capture_output=True,text=True,timeout=30)
            self.assertEqual(second.returncode,0,second.stdout+second.stderr)
            self.assertEqual(len(list(out.rglob('attempt-*/result.json'))),len(attempts))
            changed=subprocess.run(command+['--seed','42'],capture_output=True,text=True,timeout=30)
            self.assertNotEqual(changed.returncode,0)
            self.assertIn('provenance mismatch',changed.stderr)
            print('SYNTHETIC_RIVER_TIMING '+json.dumps(dict(wall_seconds=elapsed,
                  completed=summary['completed'],attempts=len(attempts),
                  timings=summary.get('timing'),estimate=summary.get('upper_bound'))))


if __name__ == '__main__':
    unittest.main()
