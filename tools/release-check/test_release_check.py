import json
import unittest

import release_check as rc


class ReleaseCheckTests(unittest.TestCase):
    def test_privacy_and_live_only(self):
        row = {"version": 1, "outcome": "inserted", "transcript": "PRIVATE",
               "audio": "PRIVATE", "stages_ms": {"release_to_inserted": 12}}
        self.assertEqual(rc.timing('prefix pipeline_timing ' + json.dumps(row)), ("inserted", 12))
        row["outcome"] = "offline_sink"
        self.assertEqual(rc.timing('pipeline_timing ' + json.dumps(row)), ("not_inserted", None))
        self.assertIsNone(rc.timing('transcript PRIVATE'))

    def test_bad_timings(self):
        for value in (True, -1, float('nan'), float('inf'), "12", None):
            row = {"version": 1, "outcome": "inserted", "stages_ms": {"release_to_inserted": value}}
            self.assertEqual(rc.timing('pipeline_timing ' + json.dumps(row)), ("invalid", None))
        for payload in ('[]', 'null', '{}', '{bad'):
            self.assertEqual(rc.timing('pipeline_timing ' + payload), ("invalid", None))

    def test_nearest_rank(self):
        self.assertEqual(rc.stats(list(range(1, 21))), {"n": 20, "median_ms": 10.5, "p95_ms": 19})
        self.assertIsNone(rc.stats([]))

    def test_top_discards_initial_and_unrelated_processes(self):
        text = 'PID %CPU MEM POWER\n1 99.0 1G 10\n2 0 99M 0\n1 0.2 1024K+ 0.1\n'
        self.assertEqual(rc.top_rows(text, [1]), {1: [{"cpu_percent": .2, "mem_mib": 1, "power_indicator": .1}]})
        self.assertEqual(rc.top_rows('1 NaN 1M 0\n', [1]), {1: []})

    def test_energy_allowlist_and_completeness(self):
        text = 'SECRET transcript\nCPU Power: 10 mW\nCPU Power: 30 mW\nGPU Power: 2 mW\n'
        self.assertEqual(rc.energy_rows(text, 2), {"cpu_mean_mw": 20})
        self.assertEqual(rc.energy_rows('password required', 2), {})

    def test_no_false_pass_without_live_baseline(self):
        report = {"latency": rc.stats([1] * 20), "minimum_samples": 20,
                  "invalid": 0, "not_inserted": 0, "idle": None, "energy": None}
        self.assertEqual(rc.summary(report)[0], 'INCOMPLETE')
        report['not_inserted'] = 1
        self.assertEqual(rc.summary(report)[0], 'FAIL')

    def test_matched_pair_and_regression(self):
        report = {"latency": rc.stats([1] * 20), "minimum_samples": 20,
                  "invalid": 0, "not_inserted": 0,
                  "idle": {"total_mean": {"mem_mib": 100, "cpu_percent": 0, "power_indicator": 0}},
                  "energy": {"cpu_mean_mw": 10, "gpu_mean_mw": 0, "ane_mean_mw": 0}}
        baseline = json.loads(json.dumps(report))
        self.assertEqual(rc.summary(report, baseline)[0], 'PASS')
        report['latency']['p95_ms'] = 2
        self.assertEqual(rc.summary(report, baseline)[0], 'FAIL')


if __name__ == '__main__':
    unittest.main()
