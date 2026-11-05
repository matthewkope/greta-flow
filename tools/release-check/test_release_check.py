import argparse
import json
from pathlib import Path
import plistlib
import queue
import tempfile
import threading
from unittest import mock
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

    def test_capture_keeps_events_arriving_during_pause(self):
        # Exercise the actual reader thread and prompt transitions without an app,
        # microphone, or real resource sampler. Acknowledge each line only after
        # drain() has processed it, so this test does not depend on sleeps.
        class Pipe:
            def __init__(self):
                self.pending = queue.Queue()
                self.ack = None

            def readline(self, _limit):
                if self.ack:
                    self.ack.set()
                data, self.ack = self.pending.get(timeout=5)
                return data

            def send(self, row):
                ack = threading.Event()
                self.pending.put((("pipeline_timing " + json.dumps(row) + "\n").encode(), ack))
                if not ack.wait(5):
                    raise AssertionError("reader did not process event")

            def close(self):
                pass

        for outcome in ("recognition_failed", "recoverable", "cancelled_or_stale", "inserted"):
            with self.subTest(outcome=outcome), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                macos = root / "Test.app/Contents/MacOS"
                macos.mkdir(parents=True)
                (macos / "test-app").write_bytes(b"fixture")
                (macos.parent / "Info.plist").write_bytes(plistlib.dumps({"CFBundleExecutable": "test-app"}))
                pipe = Pipe()
                app = mock.Mock(pid=123, stderr=pipe)
                app.poll.return_value = None
                app.terminate.side_effect = lambda: pipe.pending.put((b"", None))
                args = argparse.Namespace(app=root / "Test.app", output=root / "report.json",
                                          baseline=None, minimum_samples=1, idle_seconds=5)
                prompts = 0

                def respond(_prompt):
                    nonlocal prompts
                    prompts += 1
                    if prompts == 2:
                        pipe.send({"version": 1, "outcome": "inserted",
                                   "stages_ms": {"release_to_inserted": 1}})
                    elif prompts == 3:
                        pipe.send({"version": 1, "outcome": outcome,
                                   "stages_ms": {"release_to_inserted": 2},
                                   "transcript": "PRIVATE"})
                    return ""

                with mock.patch.object(rc.subprocess, "Popen", return_value=app), \
                        mock.patch.object(rc, "run", return_value=""), \
                        mock.patch.object(rc, "child_pid", return_value=456), \
                        mock.patch.dict(rc.os.environ, {}, clear=True), \
                        mock.patch("builtins.input", side_effect=respond), \
                        mock.patch("builtins.print"):
                    code = rc.capture(args)
                raw = args.output.read_text()
                report = json.loads(raw)
                self.assertNotIn("PRIVATE", raw)
                if outcome == "inserted":
                    self.assertEqual(report["release_to_inserted_ms"], [1, 2])
                    self.assertEqual(report["latency"]["n"], 2)
                    self.assertEqual(code, 2)  # No resource/baseline evidence.
                else:
                    self.assertEqual(report["not_inserted"], 1)
                    self.assertEqual(report["status"], "FAIL")
                    self.assertEqual(code, 1)

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
