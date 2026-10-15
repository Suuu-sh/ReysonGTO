"""Synthetic tests only: no product ranges or source files are opened."""
import tempfile
import unittest
from pathlib import Path

from export_benchmark_ranges import (
    blocked, btn_weights, canonical, expand_class, frequency, load_source,
)


class ExportTests(unittest.TestCase):
    def test_class_frequency_not_divided_by_combo_count(self):
        raw = {'spots': [{'id': 'BTN_open', 'effective_stack_bb': 100,
                          'open_size_bb': 2.5, 'hands': [
                              {'hand': 'AKs', 'open': 50},
                              {'hand': 'AA', 'open': 25},
                              {'hand': '32o', 'open': 0}]}]}
        weights = btn_weights(raw)
        self.assertEqual(len(weights), 10)
        self.assertEqual(set(weights[c] for c in expand_class('AKs')), {0.5})
        self.assertEqual(set(weights[c] for c in expand_class('AA')), {0.25})
        self.assertEqual(sum(weights.values()), 3.5)
        self.assertEqual(len(blocked(weights, 'Ah')), 6)

    def test_canonical_classes(self):
        self.assertEqual(canonical('As2c'), '2cAs')
        self.assertEqual([len(expand_class(h)) for h in ('AA', 'AKs', 'AKo')], [6, 4, 12])
        for hand in ('AAs', 'AK', 'A2x', 'ZZ'):
            with self.assertRaises(ValueError):
                expand_class(hand)
        with self.assertRaises(ValueError):
            canonical('AhAh')

    def test_invalid_or_duplicate_frequency(self):
        for value in (-1, 1.01, float('nan'), float('inf')):
            with self.assertRaises(ValueError):
                frequency(value)
        raw = {'spots': [{'id': 'BTN_open', 'effective_stack_bb': 100,
                          'open_size_bb': 2.5, 'hands': [{'hand': 'AA', 'open': 50}] * 2}]}
        with self.assertRaises(ValueError):
            btn_weights(raw)

    def test_source_fingerprint_refuses_drift(self):
        with tempfile.TemporaryDirectory() as tmp:
            source = Path(tmp) / 'synthetic.json'
            source.write_text('{}')
            with self.assertRaisesRegex(ValueError, 'SHA256 mismatch'):
                load_source(source, '0' * 64)


if __name__ == '__main__':
    unittest.main()
